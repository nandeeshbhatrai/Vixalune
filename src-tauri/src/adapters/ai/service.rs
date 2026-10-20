//! The AI gateway's orchestration: which provider and model a request goes to, where the key comes from, how a conversation is
//! turned into a request, bounded retries, cancellation, persistence and the events the interface shows.
//!
//! Rules this file enforces, whatever the provider:
//! - a key is read from the credential store only inside the call that needs it, and never leaves this layer;
//! - the provider and model of a request are exactly what the user chose; a failure is reported, never answered by another
//!   provider or model;
//! - only models the provider listed or that a real test request proved can be used;
//! - automatic retries happen only before any text has arrived, only for transient errors, and at most twice;
//! - a reply is text: nothing a model says is ever executed.

use super::ProviderRegistry;
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::domain::{AppError, AppResult};
use crate::ports::{AiEventSink, AiProvider, AiStore, CredentialStore, StreamSink};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SYSTEM_PROMPT: &str = "You are an assistant inside Vixalune, a Windows application for learning security testing with tools the user installs. \
Help the user understand concepts, tool options and results. You cannot run commands or read the user's files, and you must never ask for passwords or API keys. \
When it matters, remind the user to test only systems they own or have written permission to test.";

/// Turns a model's raw reply into what is stored and shown. Used by the tool-aware assistant, which validates the reply against
/// the application's own facts before anything is kept.
pub trait TurnFinalizer: Send + Sync {
    fn finalize(&self, raw: &str) -> Finalized;
}

pub struct Finalized {
    /// Plain text kept as the message content (and sent back to the model as history).
    pub content: String,
    /// The structured answer, stored beside the message.
    pub turn_json: Option<String>,
    pub error: Option<AiError>,
}

/// How one request differs from a plain chat message.
#[derive(Default, Clone)]
pub struct Prepared {
    pub system: Option<String>,
    /// What the model sees as the newest user message, when it is more than what the person typed (retrieved context). The stored
    /// message is always only what the person typed.
    pub model_user_text: Option<String>,
    /// Do not stream text to the interface (the reply is structured and is shown only after validation).
    pub hide_deltas: bool,
    pub finalizer: Option<Arc<dyn TurnFinalizer>>,
    /// Sets the stored title of a new conversation when the typed text is not a good one.
    pub title: Option<String>,
}

const MODEL_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_CONCURRENT_REQUESTS: usize = 4;
const MAX_ATTEMPTS: u32 = 3;
const MAX_BACKOFF_SECS: u64 = 20;
const TITLE_CHARS: usize = 60;
const TEST_PROMPT: &str = "Reply with the single word: ok";

#[derive(Debug, Clone)]
pub struct Backoff {
    pub base_ms: u64,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { base_ms: 1000 }
    }
}

impl Backoff {
    /// Wait before attempt number `attempt` (1 = the first retry): exponential with a little jitter, never shorter than a
    /// `Retry-After` the provider asked for, never longer than the cap.
    pub fn wait(&self, attempt: u32, retry_after: Option<u64>) -> Duration {
        let exp = self.base_ms.saturating_mul(1u64 << attempt.saturating_sub(1).min(6));
        let jitter = if self.base_ms == 0 { 0 } else { (uuid::Uuid::new_v4().as_u128() % (exp / 5 + 1) as u128) as u64 };
        let ms = (exp + jitter).max(retry_after.map(|s| s * 1000).unwrap_or(0));
        Duration::from_millis(ms.min(MAX_BACKOFF_SECS * 1000))
    }
}

// ------------------------------------------------------------------ views sent to the interface

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderView {
    pub descriptor: ProviderDescriptor,
    pub config: ProviderConfig,
    pub has_key: bool,
    /// Masked description of the saved key (recognised prefix and length). The key itself is never returned.
    pub key_hint: Option<String>,
    pub status: ProviderStatus,
    pub active: bool,
    /// Set when the credential store itself failed, so the screen can say why instead of pretending there is no key.
    pub credential_error: Option<AiErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUsage {
    /// Requests sent to providers since Vixalune started (chat requests and connection-test requests; automatic retries are not counted).
    pub requests_used: u32,
    /// 0 = no limit.
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvidersOverview {
    pub providers: Vec<ProviderView>,
    pub credential_backend: String,
    pub settings: AiSettings,
    pub session: SessionUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<ModelInfo>,
    /// `false` when the provider cannot list models (the user must type one and verify it).
    pub from_provider: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestReport {
    pub ok: bool,
    pub state: ConnectionState,
    pub steps: Vec<TestStep>,
    pub models_listed: Option<usize>,
    pub verified_model: Option<String>,
    pub error: Option<AiErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationDetail {
    pub conversation: Conversation,
    pub messages: Vec<StoredMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendInput {
    pub conversation_id: String,
    pub text: String,
    pub provider: String,
    pub model: String,
    /// The person confirmed sending a request that is larger than their confirmation threshold.
    #[serde(default)]
    pub confirm_large: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendAck {
    pub request_id: String,
    pub conversation_id: String,
    pub user_message: StoredMessage,
    pub assistant_message_id: String,
    /// Values in the typed text that looked like secrets (passwords, tokens, private keys…) and were masked before the text was
    /// stored or sent.
    #[serde(default)]
    pub redacted_values: usize,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn ai<T>(r: AiResult<T>) -> AppResult<T> {
    r.map_err(AppError::Ai)
}

// ------------------------------------------------------------------ the service

pub struct AiService {
    registry: Arc<ProviderRegistry>,
    creds: Arc<dyn CredentialStore>,
    store: Arc<dyn AiStore>,
    backoff: Backoff,
    running: Mutex<HashMap<String, (String, CancellationToken)>>,
    models: Mutex<HashMap<String, (Instant, Vec<ModelInfo>)>>,
    session_requests: AtomicU32,
}

/// Forwards provider events to the interface and remembers whether any text has arrived (which ends the right to retry).
struct ServiceSink {
    events: Arc<dyn AiEventSink>,
    store: Arc<dyn AiStore>,
    request_id: String,
    conversation_id: String,
    message_id: String,
    text_seen: AtomicBool,
    /// The assistant message as written so far (what the user has seen). Saved every couple of seconds so a crash loses little.
    message: Mutex<StoredMessage>,
    last_flush: Mutex<Instant>,
    hide_deltas: bool,
}

impl ServiceSink {
    fn send(&self, kind: AiUiEventKind) {
        self.events.emit(AiUiEvent { request_id: self.request_id.clone(), conversation_id: self.conversation_id.clone(), message_id: self.message_id.clone(), kind });
    }

    fn written(&self) -> StoredMessage {
        self.message.lock().unwrap().clone()
    }
}

impl StreamSink for ServiceSink {
    fn event(&self, e: &StreamEvent) {
        match e {
            StreamEvent::TextDelta { text } => {
                self.text_seen.store(true, Ordering::SeqCst);
                {
                    let mut m = self.message.lock().unwrap();
                    if m.content.chars().count() + text.chars().count() <= MAX_RESPONSE_CHARS {
                        m.content.push_str(text);
                    }
                    let mut last = self.last_flush.lock().unwrap();
                    if last.elapsed() > Duration::from_millis(1500) {
                        *last = Instant::now();
                        let _ = self.store.update_message(&m);
                    }
                }
                if !self.hide_deltas {
                    self.send(AiUiEventKind::Delta { text: text.clone() });
                }
            }
            StreamEvent::Usage { usage } => self.send(AiUiEventKind::Usage { usage: usage.clone() }),
            _ => {}
        }
    }
}

struct NullSink;
impl StreamSink for NullSink {
    fn event(&self, _: &StreamEvent) {}
}

impl AiService {
    pub fn new(registry: Arc<ProviderRegistry>, creds: Arc<dyn CredentialStore>, store: Arc<dyn AiStore>) -> Arc<Self> {
        Self::with_backoff(registry, creds, store, Backoff::default())
    }

    pub fn with_backoff(registry: Arc<ProviderRegistry>, creds: Arc<dyn CredentialStore>, store: Arc<dyn AiStore>, backoff: Backoff) -> Arc<Self> {
        Arc::new(Self { registry, creds, store, backoff, running: Mutex::new(HashMap::new()), models: Mutex::new(HashMap::new()), session_requests: AtomicU32::new(0) })
    }

    /// Local-only mode: refuse anything that would contact a cloud AI provider.
    pub fn ensure_cloud_allowed(&self) -> AppResult<()> {
        if self.store.get_settings()?.local_only {
            return Err(AppError::Ai(AiError::new(AiErrorKind::LocalOnly, "local-only mode is on, so no request was sent to an AI provider")));
        }
        Ok(())
    }

    /// Count one billable request against the per-session limit, or refuse it.
    fn reserve_request(&self, settings: &AiSettings) -> AppResult<()> {
        if settings.session_request_limit == 0 {
            self.session_requests.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        let used = self.session_requests.fetch_add(1, Ordering::SeqCst);
        if used >= settings.session_request_limit {
            self.session_requests.fetch_sub(1, Ordering::SeqCst);
            return Err(AppError::Ai(AiError::new(AiErrorKind::LimitReached, format!("the limit of {} requests per session has been reached", settings.session_request_limit))));
        }
        Ok(())
    }

    fn provider(&self, id: &str) -> AppResult<Arc<dyn AiProvider>> {
        ai(self.registry.get(id))
    }

    fn config_or_default(&self, id: &str) -> AppResult<ProviderConfig> {
        Ok(self.store.get_config(id)?.unwrap_or_else(|| ProviderConfig::new(id)))
    }

    /// Start-up housekeeping: messages left "streaming" by a crash are closed, expired conversations are deleted.
    pub fn startup(&self) -> AppResult<(usize, usize)> {
        let closed = self.store.close_streaming_messages()?;
        let purged = self.purge_expired()?;
        Ok((closed, purged))
    }

    pub fn purge_expired(&self) -> AppResult<usize> {
        let days = self.store.get_settings()?.history_days;
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(i64::from(days))).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let cutoff = if days == 0 { (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true) } else { cutoff };
        self.store.purge_conversations_before(&cutoff)
    }

    // -------------------------------------------------------------- providers and credentials

    pub fn overview(&self) -> AppResult<ProvidersOverview> {
        let active = self.store.active_provider()?;
        let mut providers = Vec::new();
        for d in self.registry.descriptors() {
            providers.push(self.view(&d, active.as_deref())?);
        }
        let settings = self.store.get_settings()?;
        let session = SessionUsage { requests_used: self.session_requests.load(Ordering::SeqCst), limit: settings.session_request_limit };
        Ok(ProvidersOverview { providers, credential_backend: self.creds.backend_name().to_string(), settings, session })
    }

    fn view(&self, d: &ProviderDescriptor, active: Option<&str>) -> AppResult<ProviderView> {
        let (has_key, credential_error) = match self.creds.exists(&d.id) {
            Ok(b) => (b, None),
            Err(e) => (false, Some(AiErrorDto::from(&e))),
        };
        let mut status = self.store.get_status(&d.id)?;
        if !has_key {
            status = ProviderStatus::default();
        }
        Ok(ProviderView {
            descriptor: d.clone(),
            config: self.config_or_default(&d.id)?,
            has_key,
            key_hint: if has_key { self.store.get_key_hint(&d.id)? } else { None },
            status,
            active: has_key && active == Some(d.id.as_str()),
            credential_error,
        })
    }

    fn view_of(&self, id: &str) -> AppResult<ProviderView> {
        let d = self.provider(id)?.descriptor();
        let active = self.store.active_provider()?;
        self.view(&d, active.as_deref())
    }

    /// Save or replace a key. The key is validated for shape, written to the credential store, and read back to prove the store
    /// really holds it. Nothing about the key itself is stored anywhere else or returned.
    pub fn save_credential(&self, provider: &str, key: Secret, label: Option<String>) -> AppResult<ProviderView> {
        let p = self.provider(provider)?;
        ai(validate_api_key_format(key.expose()))?;
        ai(self.creds.set(provider, &key))?;
        match ai(self.creds.get(provider))? {
            Some(back) if back.expose() == key.expose() => {}
            _ => {
                let _ = self.creds.delete(provider);
                return Err(AppError::Ai(AiError::new(AiErrorKind::CredentialStore, "Windows Credential Manager did not keep the key, so it was not saved")));
            }
        }
        self.store.put_key_hint(provider, Some(&key.hint()))?;
        let mut cfg = self.config_or_default(provider)?;
        if label.is_some() {
            cfg.label = label.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
        }
        ai(p.validate_config(&cfg))?;
        self.store.put_config(&cfg)?;
        // A new key says nothing about the old test results.
        self.store.put_status(provider, &ProviderStatus::default())?;
        self.models.lock().unwrap().remove(provider);
        if self.store.active_provider()?.is_none() {
            self.store.set_active_provider(Some(provider))?;
        }
        self.view_of(provider)
    }

    pub fn delete_credential(&self, provider: &str) -> AppResult<ProviderView> {
        self.provider(provider)?;
        ai(self.creds.delete(provider))?;
        self.store.put_key_hint(provider, None)?;
        self.store.put_status(provider, &ProviderStatus::default())?;
        self.models.lock().unwrap().remove(provider);
        // The active provider is cleared, not silently replaced by another one.
        if self.store.active_provider()?.as_deref() == Some(provider) {
            self.store.set_active_provider(None)?;
        }
        self.view_of(provider)
    }

    pub fn update_config(&self, config: ProviderConfig) -> AppResult<ProviderView> {
        let p = self.provider(&config.provider)?;
        ai(p.validate_config(&config))?;
        if let Some(m) = &config.default_model {
            self.require_usable_model(&config.provider, m)?;
        }
        self.store.put_config(&config)?;
        self.view_of(&config.provider)
    }

    pub fn set_active(&self, provider: &str) -> AppResult<ProviderView> {
        self.provider(provider)?;
        if !ai(self.creds.exists(provider))? {
            return Err(AppError::Ai(AiError::new(AiErrorKind::Configuration, "save an API key for this provider before making it active")));
        }
        self.store.set_active_provider(Some(provider))?;
        self.view_of(provider)
    }

    pub fn update_settings(&self, s: AiSettings) -> AppResult<AiSettings> {
        s.validate()?;
        self.store.put_settings(&s)?;
        self.purge_expired()?;
        Ok(s)
    }

    fn key(&self, provider: &str) -> AppResult<Secret> {
        match ai(self.creds.get(provider))? {
            Some(k) => Ok(k),
            None => Err(AppError::Ai(AiError::new(AiErrorKind::Configuration, "no API key is saved for this provider"))),
        }
    }

    // -------------------------------------------------------------- models and testing

    fn cached_models(&self, provider: &str) -> Option<Vec<ModelInfo>> {
        let m = self.models.lock().unwrap();
        m.get(provider).filter(|(at, _)| at.elapsed() < MODEL_CACHE_TTL).map(|(_, v)| v.clone())
    }

    pub async fn list_models(&self, provider: &str) -> AppResult<ModelList> {
        let p = self.provider(provider)?;
        if p.descriptor().capabilities.model_listing {
            self.ensure_cloud_allowed()?;
        }
        let verified = self.store.get_status(provider)?.verified_models;
        let cfg = self.config_or_default(provider)?;
        if !p.descriptor().capabilities.model_listing {
            let models = verified.iter().map(|id| ModelInfo { id: id.clone(), display_name: None, context_window: None, max_output_tokens: None, supports_tools: None, pricing: None, source: ModelSource::Verified }).collect();
            return Ok(ModelList { models, from_provider: false, note: "This provider cannot list its models. Type the model name and use Test connection to verify it.".into() });
        }
        let key = self.key(provider)?;
        let timeout = Duration::from_secs(u64::from(cfg.timeout_secs));
        let cancel = CancellationToken::new();
        let listed = match self.cached_models(provider) {
            Some(l) => l,
            None => {
                let l = self.retry_simple(|| p.list_models(&key, timeout, &cancel), &cancel).await.map_err(|e| AppError::Ai(e.scrubbed(&key)))?;
                self.models.lock().unwrap().insert(provider.to_string(), (Instant::now(), l.clone()));
                l
            }
        };
        let mut models = listed;
        for v in verified {
            if !models.iter().any(|m| m.id == v) {
                models.push(ModelInfo { id: v, display_name: None, context_window: None, max_output_tokens: None, supports_tools: None, pricing: None, source: ModelSource::Verified });
            }
        }
        Ok(ModelList { models, from_provider: true, note: "Listed by the provider. A model in the list may still be refused for your account; Test connection checks the one you choose.".into() })
    }

    /// The model must come from the provider's own list or have passed a real test request.
    fn require_usable_model(&self, provider: &str, model: &str) -> AppResult<()> {
        let bad = || AppError::Ai(AiError::new(AiErrorKind::Configuration, "that model is not in the provider's list and has not been verified; choose a listed model, or use Test connection with this name first"));
        if !valid_model_id(model) {
            return Err(AppError::Ai(AiError::new(AiErrorKind::ModelNotFound, "choose a valid model name")));
        }
        if self.store.get_status(provider)?.verified_models.iter().any(|m| m == model) {
            return Ok(());
        }
        match self.cached_models(provider) {
            Some(l) if l.iter().any(|m| m.id == model) => Ok(()),
            _ => Err(bad()),
        }
    }

    async fn retry_simple<T, F, Fut>(&self, mut f: F, cancel: &CancellationToken) -> AiResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = AiResult<T>>,
    {
        let mut attempt = 0;
        loop {
            match f().await {
                Err(e) if e.retryable() && attempt + 1 < MAX_ATTEMPTS => {
                    attempt += 1;
                    self.sleep(self.backoff.wait(attempt, e.retry_after_secs), cancel).await?;
                }
                other => return other,
            }
        }
    }

    async fn sleep(&self, d: Duration, cancel: &CancellationToken) -> AiResult<()> {
        let end = Instant::now() + d;
        while Instant::now() < end {
            if cancel.is_cancelled() {
                return Err(AiError::cancelled());
            }
            tokio::time::sleep(Duration::from_millis(25).min(end.saturating_duration_since(Instant::now()))).await;
        }
        Ok(())
    }

    /// Test the saved key against the provider. Success is reported only for what was actually proven:
    /// the key was accepted by an authenticated request; and, when a model is given, a real one-line request with that model worked.
    pub async fn test_connection(&self, provider: &str, model: Option<String>) -> AppResult<TestReport> {
        self.ensure_cloud_allowed()?;
        let p = self.provider(provider)?;
        let key = self.key(provider)?;
        let cfg = self.config_or_default(provider)?;
        let timeout = Duration::from_secs(u64::from(cfg.timeout_secs));
        let cancel = CancellationToken::new();
        let mut steps = Vec::new();
        let mut status = self.store.get_status(provider)?;
        status.steps.clear();
        status.tested_at = Some(now());
        let mut models_listed = None;
        let mut verified_model = None;
        let mut error: Option<AiError> = None;

        match self.retry_simple(|| p.check_auth(&key, timeout, &cancel), &cancel).await {
            Ok(()) => steps.push(TestStep { name: "Key accepted by the provider".into(), ok: true, detail: "An authenticated request succeeded.".into() }),
            Err(e) => {
                let e = e.scrubbed(&key);
                steps.push(TestStep { name: "Key accepted by the provider".into(), ok: false, detail: e.message.clone() });
                error = Some(e);
            }
        }

        if error.is_none() && p.descriptor().capabilities.model_listing {
            match self.retry_simple(|| p.list_models(&key, timeout, &cancel), &cancel).await {
                Ok(l) => {
                    models_listed = Some(l.len());
                    steps.push(TestStep { name: "Model list".into(), ok: true, detail: format!("{} models listed by the provider.", l.len()) });
                    self.models.lock().unwrap().insert(provider.to_string(), (Instant::now(), l));
                }
                Err(e) => steps.push(TestStep { name: "Model list".into(), ok: false, detail: format!("{} You can still type a model name and verify it.", e.scrubbed(&key).message) }),
            }
        }

        let chosen = model.or_else(|| cfg.default_model.clone());
        if error.is_none() {
            if let Some(m) = chosen {
                if !valid_model_id(&m) {
                    return Err(AppError::Ai(AiError::new(AiErrorKind::ModelNotFound, "choose a valid model name")));
                }
                let req = ChatRequest { model: m.clone(), system: None, messages: vec![ChatMessage::user(TEST_PROMPT)], tools: vec![], params: GenerationParams { temperature: None, max_output_tokens: Some(16) }, idle_timeout_secs: cfg.timeout_secs };
                self.reserve_request(&self.store.get_settings()?)?;
                let mut tries = 0;
                let result = loop {
                    let r = p.chat_stream(&key, &req, &cfg, &NullSink, &cancel).await;
                    match r {
                        Err(e) if e.retryable() && tries + 1 < MAX_ATTEMPTS => {
                            tries += 1;
                            self.sleep(self.backoff.wait(tries, e.retry_after_secs), &cancel).await.ok();
                        }
                        other => break other,
                    }
                };
                match result {
                    Ok(o) if !o.text.trim().is_empty() || o.finish.is_some() => {
                        steps.push(TestStep { name: format!("Test request with {m}"), ok: true, detail: match o.usage.total_tokens { Some(t) => format!("The model answered ({t} tokens used)."), None => "The model answered.".into() } });
                        if !status.verified_models.contains(&m) {
                            status.verified_models.push(m.clone());
                        }
                        verified_model = Some(m);
                    }
                    Ok(_) => {
                        let e = AiError::new(AiErrorKind::Protocol, "the model returned an empty answer");
                        steps.push(TestStep { name: format!("Test request with {m}"), ok: false, detail: e.message.clone() });
                        error = Some(e);
                    }
                    Err(e) => {
                        let e = e.scrubbed(&key);
                        steps.push(TestStep { name: format!("Test request with {m}"), ok: false, detail: e.message.clone() });
                        error = Some(e);
                    }
                }
            } else {
                steps.push(TestStep { name: "Model not yet verified".into(), ok: false, detail: "Choose a model and test again to prove it works with your key.".into() });
            }
        }

        let state = if error.is_some() {
            ConnectionState::Failed
        } else if verified_model.is_some() {
            ConnectionState::Verified
        } else {
            ConnectionState::KeyAccepted
        };
        status.state = state;
        status.models_listed = models_listed.or(status.models_listed);
        status.error_code = error.as_ref().map(|e| e.kind.code().to_string());
        status.message = error.as_ref().map(|e| e.message.clone());
        status.steps = steps.clone();
        self.store.put_status(provider, &status)?;
        Ok(TestReport { ok: state == ConnectionState::Verified, state, steps, models_listed, verified_model, error: error.as_ref().map(AiErrorDto::from) })
    }

    // -------------------------------------------------------------- conversations

    pub fn create_conversation(&self, provider: Option<String>, model: Option<String>) -> AppResult<Conversation> {
        let t = now();
        let c = Conversation { id: uuid::Uuid::new_v4().to_string(), title: "New conversation".into(), provider, model, created_at: t.clone(), updated_at: t };
        self.store.create_conversation(&c)?;
        Ok(c)
    }

    pub fn list_conversations(&self) -> AppResult<Vec<Conversation>> {
        self.store.list_conversations(200)
    }

    pub fn get_conversation(&self, id: &str) -> AppResult<ConversationDetail> {
        Ok(ConversationDetail { conversation: self.store.get_conversation(id)?, messages: self.store.list_messages(id)? })
    }

    pub fn delete_conversation(&self, id: &str) -> AppResult<()> {
        if self.running.lock().unwrap().values().any(|(c, _)| c == id) {
            return Err(AppError::Busy("a response is still being written in this conversation; stop it first".into()));
        }
        self.store.delete_conversation(id)
    }

    pub fn delete_all_conversations(&self) -> AppResult<usize> {
        if !self.running.lock().unwrap().is_empty() {
            return Err(AppError::Busy("a response is still being written; stop it first".into()));
        }
        self.store.delete_all_conversations()
    }

    // -------------------------------------------------------------- sending

    pub fn cancel(&self, request_id: &str) -> AppResult<()> {
        match self.running.lock().unwrap().get(request_id) {
            Some((_, t)) => {
                t.cancel();
                Ok(())
            }
            None => Err(AppError::NotFound("that request (it may already have finished)".into())),
        }
    }

    pub fn cancel_all(&self) -> usize {
        let r = self.running.lock().unwrap();
        r.values().for_each(|(_, t)| t.cancel());
        r.len()
    }

    pub fn active_requests(&self) -> usize {
        self.running.lock().unwrap().len()
    }

    /// The checks on typed text that do not need a key: not empty, not too long, no credential-shaped text.
    pub fn check_user_text(&self, text: &str) -> AppResult<()> {
        let bad = |m: &str| Err(AppError::Ai(AiError::new(AiErrorKind::InvalidRequest, m)));
        if text.trim().is_empty() {
            return bad("write a message first");
        }
        if text.chars().count() > MAX_USER_MESSAGE_CHARS {
            return Err(AppError::Ai(AiError::new(AiErrorKind::TooLarge, format!("a message can be at most {MAX_USER_MESSAGE_CHARS} characters"))));
        }
        if looks_like_credential(text) {
            return bad("your message looks like it contains an API key or password. Remove it: messages are sent to the provider and kept in your history");
        }
        Ok(())
    }

    /// Refuse a message that looks like it carries a credential: it would be sent to the provider and stored in the history.
    fn check_message_text(&self, text: &str, key: &Secret) -> AppResult<()> {
        let bad = |m: &str| Err(AppError::Ai(AiError::new(AiErrorKind::InvalidRequest, m)));
        if text.trim().is_empty() {
            return bad("write a message first");
        }
        if text.chars().count() > MAX_USER_MESSAGE_CHARS {
            return Err(AppError::Ai(AiError::new(AiErrorKind::TooLarge, format!("a message can be at most {MAX_USER_MESSAGE_CHARS} characters"))));
        }
        if text.contains(key.expose()) || looks_like_credential(text) {
            return bad("your message looks like it contains an API key or password. Remove it: messages are sent to the provider and kept in your history");
        }
        Ok(())
    }

    fn build_request(&self, history: &[StoredMessage], new_user: &ChatMessage, model: &str, cfg: &ProviderConfig, system: &str, max_chars: usize) -> ChatRequest {
        let mut msgs: Vec<ChatMessage> = history
            .iter()
            .filter(|m| m.status == MessageStatus::Complete && !m.content.trim().is_empty() && matches!(m.role, Role::User | Role::Assistant))
            .map(|m| ChatMessage { role: m.role, content: m.content.clone(), tool_calls: vec![], tool_call_id: None })
            .collect();
        msgs.push(new_user.clone());
        // Keep the conversation within the size bound by dropping the oldest messages; start on a user turn.
        let mut total: usize = msgs.iter().map(|m| m.content.chars().count()).sum::<usize>() + system.len();
        while total > max_chars.min(MAX_CONTEXT_CHARS) && msgs.len() > 1 {
            total -= msgs.remove(0).content.chars().count();
        }
        while msgs.len() > 1 && msgs[0].role != Role::User {
            msgs.remove(0);
        }
        // Alternate roles cleanly: a failed assistant turn left two user messages in a row; providers reject that.
        let mut merged: Vec<ChatMessage> = Vec::new();
        for m in msgs {
            match merged.last_mut() {
                Some(last) if last.role == m.role => {
                    last.content.push_str("\n\n");
                    last.content.push_str(&m.content);
                }
                _ => merged.push(m),
            }
        }
        ChatRequest { model: model.to_string(), system: Some(system.to_string()), messages: merged, tools: vec![], params: GenerationParams { temperature: cfg.temperature, max_output_tokens: Some(cfg.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)) }, idle_timeout_secs: cfg.timeout_secs }
    }

    /// Send a message and stream the answer. Returns as soon as the request has been accepted for sending; the answer arrives as
    /// events and is stored when it ends.
    pub fn send_message(self: &Arc<Self>, input: SendInput, events: Arc<dyn AiEventSink>) -> AppResult<SendAck> {
        self.send_prepared(input, Prepared::default(), events)
    }

    /// Like `send_message`, with the system prompt, the model-visible text and the finalizer supplied by the caller.
    pub fn send_prepared(self: &Arc<Self>, mut input: SendInput, prep: Prepared, events: Arc<dyn AiEventSink>) -> AppResult<SendAck> {
        self.ensure_cloud_allowed()?;
        let provider = self.provider(&input.provider)?;
        let key = self.key(&input.provider)?;
        self.check_message_text(&input.text, &key)?;
        // Pasted output may carry secrets (passwords, tokens, private keys). They are masked before the text is stored or sent.
        let (clean, redacted_values) = redact_secrets_in_text(&input.text);
        input.text = clean;
        let settings = self.store.get_settings()?;
        let cfg = self.config_or_default(&input.provider)?;
        ai(provider.validate_config(&cfg))?;
        self.require_usable_model(&input.provider, &input.model)?;
        let conversation = self.store.get_conversation(&input.conversation_id)?;
        {
            let running = self.running.lock().unwrap();
            if running.values().any(|(c, _)| c == &conversation.id) {
                return Err(AppError::Busy("a response is already being written in this conversation".into()));
            }
            if running.len() >= MAX_CONCURRENT_REQUESTS {
                return Err(AppError::Busy("several responses are already being written; wait for one to finish".into()));
            }
        }
        let history = self.store.list_messages(&conversation.id)?;
        let new_user = ChatMessage::user(prep.model_user_text.clone().unwrap_or_else(|| input.text.clone()));
        let system = prep.system.clone().unwrap_or_else(|| SYSTEM_PROMPT.to_string());
        let request = self.build_request(&history, &new_user, &input.model, &cfg, &system, settings.max_request_chars as usize);
        ai(provider.validate_request(&request))?;
        // Request size controls, before anything is stored or sent.
        let size: usize = request.messages.iter().map(|m| m.content.chars().count()).sum::<usize>() + request.system.as_deref().map_or(0, |s| s.chars().count());
        if size > settings.max_request_chars as usize {
            return Err(AppError::Ai(AiError::new(AiErrorKind::TooLarge, format!("this request is about {size} characters, more than the {} you allow in Settings, AI Providers", settings.max_request_chars))));
        }
        if settings.confirm_large_chars > 0 && size > settings.confirm_large_chars as usize && !input.confirm_large {
            return Err(AppError::Ai(AiError::new(AiErrorKind::NeedsConfirmation, format!("this request is large: about {size} characters (roughly {} tokens) will be sent to {} and may cost more than usual", size / 4, provider.descriptor().display_name))));
        }
        self.reserve_request(&settings)?;

        let t = now();
        let user_message = self.store.append_message(&StoredMessage { id: uuid::Uuid::new_v4().to_string(), conversation_id: conversation.id.clone(), seq: 0, role: Role::User, content: input.text.clone(), status: MessageStatus::Complete, error_code: None, error_message: None, provider: None, model: None, usage: None, created_at: t.clone() })?;
        let assistant = self.store.append_message(&StoredMessage { id: uuid::Uuid::new_v4().to_string(), conversation_id: conversation.id.clone(), seq: 0, role: Role::Assistant, content: String::new(), status: MessageStatus::Streaming, error_code: None, error_message: None, provider: Some(input.provider.clone()), model: Some(input.model.clone()), usage: None, created_at: t.clone() })?;
        let mut conv = conversation.clone();
        conv.provider = Some(input.provider.clone());
        conv.model = Some(input.model.clone());
        conv.updated_at = t;
        if history.is_empty() {
            conv.title = prep.title.clone().unwrap_or_else(|| input.text.clone()).split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TITLE_CHARS).collect();
        }
        self.store.update_conversation(&conv)?;

        let request_id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        self.running.lock().unwrap().insert(request_id.clone(), (conv.id.clone(), cancel.clone()));
        let ack = SendAck { request_id: request_id.clone(), conversation_id: conv.id.clone(), user_message, assistant_message_id: assistant.id.clone(), redacted_values };

        let me = self.clone();
        let sink = Arc::new(ServiceSink { events, store: self.store.clone(), request_id: request_id.clone(), conversation_id: conv.id.clone(), message_id: assistant.id.clone(), text_seen: AtomicBool::new(false), message: Mutex::new(assistant.clone()), last_flush: Mutex::new(Instant::now()), hide_deltas: prep.hide_deltas });
        let finalizer = prep.finalizer.clone();
        let provider_id = input.provider.clone();
        let model = input.model.clone();
        tokio::spawn(async move {
            me.run_request(provider, provider_id, model, key, request, cfg, sink, cancel, request_id, finalizer).await;
        });
        Ok(ack)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_request(&self, provider: Arc<dyn AiProvider>, provider_id: String, model: String, key: Secret, request: ChatRequest, cfg: ProviderConfig, sink: Arc<ServiceSink>, cancel: CancellationToken, request_id: String, finalizer: Option<Arc<dyn TurnFinalizer>>) {
        let mut message = sink.written();
        sink.send(AiUiEventKind::Started { provider: provider_id.clone(), model: model.clone() });
        let mut attempt = 0u32;
        let result = loop {
            attempt += 1;
            let r = provider.chat_stream(&key, &request, &cfg, sink.as_ref(), &cancel).await;
            match r {
                Err(e) if e.retryable() && attempt < MAX_ATTEMPTS && !sink.text_seen.load(Ordering::SeqCst) && !cancel.is_cancelled() => {
                    let wait = self.backoff.wait(attempt, e.retry_after_secs);
                    sink.send(AiUiEventKind::Retrying { attempt, wait_secs: wait.as_secs().max(1), reason: e.scrubbed(&key).message });
                    if self.sleep(wait, &cancel).await.is_err() {
                        break Err(AiError::cancelled());
                    }
                }
                other => break other,
            }
        };

        // What the stream produced is in the sink only as events; the outcome carries the assembled text.
        let (status, finish, usage, error) = match result {
            Ok(o) => {
                message.content = o.text.clone();
                let mut status = if o.finish.is_none() && o.text.is_empty() { MessageStatus::Failed } else { MessageStatus::Complete };
                let mut err = if status == MessageStatus::Failed { Some(AiError::new(AiErrorKind::Protocol, "the provider ended the response without any content")) } else { None };
                if let (Some(f), MessageStatus::Complete) = (finalizer.as_ref(), status) {
                    let done = f.finalize(&o.text);
                    match (done.error, done.turn_json) {
                        (None, Some(json)) => {
                            message.content = done.content;
                            if let Err(e) = self.store.put_turn(&message.id, &message.conversation_id, &json) {
                                tracing::warn!("could not store the assistant answer: {e}");
                            }
                        }
                        (e, _) => {
                            message.content = String::new();
                            status = MessageStatus::Failed;
                            err = Some(e.unwrap_or_else(|| AiError::new(AiErrorKind::Protocol, "the answer could not be checked")));
                        }
                    }
                }
                let mut finish = o.finish.clone();
                if o.truncated_by_limit {
                    finish = Some(FinishReason::Length);
                }
                (status, finish, o.usage, err)
            }
            Err(e) => {
                let e = e.scrubbed(&key);
                let status = if e.kind == AiErrorKind::Cancelled {
                    MessageStatus::Cancelled
                } else if sink.text_seen.load(Ordering::SeqCst) && matches!(e.kind, AiErrorKind::Network | AiErrorKind::Timeout | AiErrorKind::Server | AiErrorKind::Overloaded | AiErrorKind::Protocol) {
                    MessageStatus::Interrupted
                } else {
                    MessageStatus::Failed
                };
                (status, None, Usage::default(), Some(e))
            }
        };
        // The text of an interrupted or cancelled reply is kept: it is what the user saw.
        if matches!(status, MessageStatus::Interrupted | MessageStatus::Cancelled | MessageStatus::Failed) && finalizer.is_none() {
            message.content = sink.written().content;
        }
        if finalizer.is_some() && status != MessageStatus::Complete {
            // Raw structured text that failed validation is not kept or shown.
            message.content = String::new();
        }
        message.status = status;
        message.error_code = error.as_ref().map(|e| e.kind.code().to_string());
        message.error_message = error.as_ref().map(|e| e.message.clone());
        message.usage = if usage.is_empty() { None } else { Some(usage.clone()) };
        if let Err(e) = self.store.update_message(&message) {
            tracing::warn!("could not store the AI reply: {e}");
        }
        sink.send(AiUiEventKind::Finished { status, finish_reason: finish, usage, error: error.as_ref().map(AiErrorDto::from) });
        self.running.lock().unwrap().remove(&request_id);
    }

    /// Re-send the last user message after a failed, stopped or interrupted answer. This is always an explicit user action.
    pub fn retry_last(self: &Arc<Self>, conversation_id: &str, provider: String, model: String, events: Arc<dyn AiEventSink>) -> AppResult<SendAck> {
        let text = self.take_last_failed_question(conversation_id)?;
        self.send_message(SendInput { conversation_id: conversation_id.to_string(), text, provider, model, confirm_large: false }, events)
    }

    /// Remove the last failed, stopped or cut-off answer and the question it answered; return the question's text so it can be asked
    /// again. Always the result of an explicit user action.
    pub fn take_last_failed_question(&self, conversation_id: &str) -> AppResult<String> {
        let messages = self.store.list_messages(conversation_id)?;
        let Some(last) = messages.last() else { return Err(AppError::NotFound("a message to retry".into())) };
        if last.role != Role::Assistant || last.status == MessageStatus::Complete || last.status == MessageStatus::Streaming {
            return Err(AppError::Validation("only an answer that failed, was stopped or was cut off can be retried".into()));
        }
        let Some(user) = messages.iter().rev().find(|m| m.role == Role::User) else { return Err(AppError::NotFound("a message to retry".into())) };
        let text = user.content.clone();
        let (user_id, assistant_id) = (user.id.clone(), last.id.clone());
        self.store.delete_message(&assistant_id)?;
        self.store.delete_message(&user_id)?;
        Ok(text)
    }

    /// Store a question and an answer that the application produced itself, without asking a model (for example when details are
    /// missing). The events keep the interface path the same as for a model reply.
    pub fn record_local_turn(&self, conversation_id: &str, user_text: &str, answer_text: &str, turn_json: &str, events: &dyn AiEventSink) -> AppResult<SendAck> {
        let conversation = self.store.get_conversation(conversation_id)?;
        let t = now();
        let history_empty = self.store.list_messages(conversation_id)?.is_empty();
        let user_message = self.store.append_message(&StoredMessage { id: uuid::Uuid::new_v4().to_string(), conversation_id: conversation.id.clone(), seq: 0, role: Role::User, content: user_text.to_string(), status: MessageStatus::Complete, error_code: None, error_message: None, provider: None, model: None, usage: None, created_at: t.clone() })?;
        let assistant = self.store.append_message(&StoredMessage { id: uuid::Uuid::new_v4().to_string(), conversation_id: conversation.id.clone(), seq: 0, role: Role::Assistant, content: answer_text.to_string(), status: MessageStatus::Complete, error_code: None, error_message: None, provider: None, model: None, usage: None, created_at: t.clone() })?;
        self.store.put_turn(&assistant.id, &conversation.id, turn_json)?;
        let mut conv = conversation;
        conv.updated_at = t;
        if history_empty {
            conv.title = user_text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TITLE_CHARS).collect();
        }
        self.store.update_conversation(&conv)?;
        let request_id = uuid::Uuid::new_v4().to_string();
        let ev = |kind| AiUiEvent { request_id: request_id.clone(), conversation_id: conv.id.clone(), message_id: assistant.id.clone(), kind };
        events.emit(ev(AiUiEventKind::Finished { status: MessageStatus::Complete, finish_reason: None, usage: Usage::default(), error: None }));
        Ok(SendAck { request_id, conversation_id: conv.id, user_message, assistant_message_id: assistant.id, redacted_values: 0 })
    }

    pub fn turns(&self, conversation_id: &str) -> AppResult<Vec<(String, String)>> {
        self.store.list_turns(conversation_id)
    }

    pub fn settings(&self) -> AppResult<AiSettings> {
        self.store.get_settings()
    }

    pub fn conversation(&self, id: &str) -> AppResult<Conversation> {
        self.store.get_conversation(id)
    }

    pub fn history(&self, conversation_id: &str) -> AppResult<Vec<StoredMessage>> {
        self.store.list_messages(conversation_id)
    }

    /// Whether a provider has a key and a usable model; used before spending effort on retrieval.
    pub fn ready_for(&self, provider: &str, model: &str) -> AppResult<()> {
        self.provider(provider)?;
        self.key(provider)?;
        self.require_usable_model(provider, model)
    }
}
