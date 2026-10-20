//! The AI service against a scripted provider, an in-memory credential store and a real SQLite store: key handling, model
//! selection, connection tests, streaming, persistence, retries, cancellation and retention.

use super::credentials::MemoryCredentialStore;
use super::service::*;
use super::store::SqliteAiStore;
use super::ProviderRegistry;
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::domain::AppError;
use crate::ports::{AiEventSink, AiProvider, AiStore, CredentialStore, StreamSink};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY: &str = "sk-test-0123456789abcdefghij";

#[derive(Clone)]
enum Script {
    Reply { chunks: Vec<&'static str>, usage: Option<Usage>, finish: FinishReason },
    Fail(AiError),
    /// Text first, then the connection breaks.
    FailAfterText { chunks: Vec<&'static str>, error: AiError },
    /// Text, then wait until cancelled.
    Hang { chunks: Vec<&'static str> },
}

struct Fake {
    id: String,
    listing: bool,
    models: Vec<&'static str>,
    auth: Mutex<Option<AiError>>,
    script: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<ChatRequest>>,
    keys_seen: Mutex<Vec<String>>,
    list_calls: AtomicUsize,
}

impl Fake {
    fn new(id: &str, listing: bool, models: Vec<&'static str>) -> Arc<Self> {
        Arc::new(Self { id: id.into(), listing, models, auth: Mutex::new(None), script: Mutex::new(VecDeque::new()), requests: Mutex::new(vec![]), keys_seen: Mutex::new(vec![]), list_calls: AtomicUsize::new(0) })
    }
    fn push(&self, s: Script) {
        self.script.lock().unwrap().push_back(s);
    }
    fn reply(text: &'static str) -> Script {
        Script::Reply { chunks: vec![text], usage: Some(Usage { input_tokens: Some(5), output_tokens: Some(2), total_tokens: Some(7), cost_usd: None }), finish: FinishReason::Stop }
    }
}

#[async_trait]
impl AiProvider for Fake {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.id.clone(),
            display_name: self.id.clone(),
            capabilities: ProviderCapabilities { streaming: true, tool_calling: false, structured_output: false, model_listing: self.listing, usage_reporting: true, cost_reporting: false, cancellation: true, context_limits: false },
            key_hint: String::new(),
            key_page: "https://example.test/keys".into(),
            docs_page: "https://example.test/docs".into(),
            billing_note: "test".into(),
            verification_note: "test double".into(),
            extra_settings: vec![],
        }
    }
    fn validate_config(&self, c: &ProviderConfig) -> AiResult<()> {
        validate_config_generic(c)
    }
    fn validate_request(&self, r: &ChatRequest) -> AiResult<()> {
        super::common::validate_common(r)
    }
    async fn check_auth(&self, key: &Secret, _t: Duration, _c: &CancellationToken) -> AiResult<()> {
        self.keys_seen.lock().unwrap().push(key.expose().to_string());
        match self.auth.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    async fn list_models(&self, _k: &Secret, _t: Duration, _c: &CancellationToken) -> AiResult<Vec<ModelInfo>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        if !self.listing {
            return Err(AiError::new(AiErrorKind::Unsupported, "no model list"));
        }
        Ok(self.models.iter().map(|m| ModelInfo { id: m.to_string(), display_name: None, context_window: None, max_output_tokens: None, supports_tools: None, pricing: None, source: ModelSource::Listed }).collect())
    }
    async fn chat_stream(&self, key: &Secret, req: &ChatRequest, _cfg: &ProviderConfig, sink: &dyn StreamSink, cancel: &CancellationToken) -> AiResult<ChatOutcome> {
        self.requests.lock().unwrap().push(req.clone());
        self.keys_seen.lock().unwrap().push(key.expose().to_string());
        let script = self.script.lock().unwrap().pop_front().unwrap_or_else(|| Fake::reply("ok"));
        let mut o = ChatOutcome::default();
        let say = |chunks: &[&'static str], o: &mut ChatOutcome| {
            for c in chunks {
                let ev = StreamEvent::TextDelta { text: c.to_string() };
                sink.event(&ev);
                o.apply(&ev);
            }
        };
        match script {
            Script::Reply { chunks, usage, finish } => {
                say(&chunks, &mut o);
                if let Some(u) = usage {
                    sink.event(&StreamEvent::Usage { usage: u.clone() });
                    o.usage = u;
                }
                o.finish = Some(finish);
                Ok(o)
            }
            Script::Fail(e) => Err(e),
            Script::FailAfterText { chunks, error } => {
                say(&chunks, &mut o);
                Err(error)
            }
            Script::Hang { chunks } => {
                say(&chunks, &mut o);
                while !cancel.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(AiError::cancelled())
            }
        }
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<AiUiEvent>>);
impl AiEventSink for Events {
    fn emit(&self, e: AiUiEvent) {
        self.0.lock().unwrap().push(e);
    }
}
impl Events {
    fn all(&self) -> Vec<AiUiEvent> {
        self.0.lock().unwrap().clone()
    }
    fn finished(&self) -> Option<AiUiEvent> {
        self.all().into_iter().find(|e| matches!(e.kind, AiUiEventKind::Finished { .. }))
    }
    async fn wait_finished(&self) -> AiUiEvent {
        for _ in 0..500 {
            if let Some(e) = self.finished() {
                return e;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the request never finished: {:?}", self.all());
    }
}

struct H {
    svc: Arc<AiService>,
    fake: Arc<Fake>,
    creds: Arc<MemoryCredentialStore>,
    store: Arc<SqliteAiStore>,
}

fn harness_with(listing: bool, models: Vec<&'static str>) -> H {
    let fake = Fake::new("fake", listing, models);
    let mut reg = ProviderRegistry::new();
    reg.register(fake.clone()).unwrap();
    let other = Fake::new("other", true, vec!["o-model"]);
    reg.register(other).unwrap();
    let creds = Arc::new(MemoryCredentialStore::default());
    let store = Arc::new(SqliteAiStore::in_memory().unwrap());
    let svc = AiService::with_backoff(Arc::new(reg), creds.clone(), store.clone(), Backoff { base_ms: 1 });
    H { svc, fake, creds, store }
}

fn harness() -> H {
    harness_with(true, vec!["model-a", "model-b"])
}

fn input(c: &Conversation, text: &str, model: &str) -> SendInput {
    SendInput { conversation_id: c.id.clone(), text: text.into(), provider: "fake".into(), model: model.into(), confirm_large: false }
}

async fn ready(h: &H) -> Conversation {
    h.svc.save_credential("fake", Secret::new(KEY.into()), Some("work".into())).unwrap();
    h.svc.list_models("fake").await.unwrap();
    h.svc.create_conversation(Some("fake".into()), Some("model-a".into())).unwrap()
}

// ------------------------------------------------------------------ credentials

#[test]
fn a_key_can_be_saved_replaced_read_back_by_the_backend_and_deleted_and_is_never_returned() {
    let h = harness();
    let v = h.svc.save_credential("fake", Secret::new(KEY.into()), Some("  work  ".into())).unwrap();
    assert!(v.has_key && v.active, "the first saved provider becomes active");
    assert_eq!(v.config.label.as_deref(), Some("work"));
    assert_eq!(h.creds.get("fake").unwrap().unwrap().expose(), KEY);
    let hint = v.key_hint.clone().unwrap();
    assert!(hint.contains("(28 characters)") && !hint.contains("0123456789"), "{hint}");
    // Nothing the interface can receive contains the key.
    let overview = serde_json::to_string(&h.svc.overview().unwrap()).unwrap();
    assert!(!overview.contains(KEY) && !overview.contains("0123456789abcdefghij"), "{overview}");
    // Replace.
    let new_key = "sk-test-REPLACEMENT-9876543210";
    let v = h.svc.save_credential("fake", Secret::new(new_key.into()), None).unwrap();
    assert_eq!(h.creds.get("fake").unwrap().unwrap().expose(), new_key);
    assert_eq!(v.config.label.as_deref(), Some("work"), "replacing a key keeps the label unless a new one is given");
    assert_eq!(v.status.state, ConnectionState::Untested, "an old test result says nothing about a new key");
    // Delete.
    let v = h.svc.delete_credential("fake").unwrap();
    assert!(!v.has_key && !v.active && v.key_hint.is_none());
    assert!(h.creds.get("fake").unwrap().is_none());
    assert_eq!(h.store.active_provider().unwrap(), None, "the active provider is cleared, not handed to another one");
    assert!(h.svc.delete_credential("fake").is_ok(), "deleting twice is harmless");
}

#[test]
fn bad_keys_and_unknown_providers_are_refused_without_echoing_the_key() {
    let h = harness();
    for bad in ["", "short", "has a space in the key 123456", "line\nbreak-1234567890", "ünïcödé-key-1234567890"] {
        let e = h.svc.save_credential("fake", Secret::new(bad.into()), None).unwrap_err();
        assert!(matches!(e, AppError::Ai(ref a) if a.kind == AiErrorKind::Configuration), "{e:?}");
        assert!(bad.len() < 12 || !e.to_string().contains(bad), "{e}");
    }
    assert!(h.creds.get("fake").unwrap().is_none(), "nothing was stored");
    assert!(h.svc.save_credential("nope", Secret::new(KEY.into()), None).is_err());
    assert!(h.svc.set_active("fake").is_err(), "no key, no active provider");
}

#[test]
fn if_credential_manager_is_unavailable_everything_fails_closed_and_nothing_is_stored_elsewhere() {
    let h = harness();
    h.creds.unavailable.store(true, Ordering::SeqCst);
    let e = h.svc.save_credential("fake", Secret::new(KEY.into()), None).unwrap_err();
    assert!(matches!(&e, AppError::Ai(a) if a.kind == AiErrorKind::CredentialStore), "{e:?}");
    assert!(e.to_string().contains("Credential Manager") && !e.to_string().contains(KEY));
    assert_eq!(e.code(), "ai_credential_store");
    assert!(!e.recovery().is_empty());
    // The database got no key, no hint and no configuration out of the failed attempt.
    assert_eq!(h.store.get_key_hint("fake").unwrap(), None);
    assert_eq!(h.store.get_config("fake").unwrap(), None);
    assert_eq!(h.store.active_provider().unwrap(), None);
    // Listing says why instead of pretending there is simply no key.
    let v = &h.svc.overview().unwrap().providers[0];
    assert!(!v.has_key);
    assert_eq!(v.credential_error.as_ref().unwrap().kind, AiErrorKind::CredentialStore);
    // Sending needs the key, so it fails closed too.
    let c = h.svc.create_conversation(None, None).unwrap();
    assert!(h.svc.send_message(input(&c, "hi", "model-a"), Arc::new(Events::default())).is_err());
}

#[tokio::test]
async fn sending_without_a_saved_key_says_so() {
    let h = harness();
    let c = h.svc.create_conversation(None, None).unwrap();
    let e = h.svc.send_message(input(&c, "hello", "model-a"), Arc::new(Events::default())).unwrap_err();
    assert!(matches!(&e, AppError::Ai(a) if a.kind == AiErrorKind::Configuration), "{e:?}");
    assert!(e.to_string().contains("no API key"));
}

// ------------------------------------------------------------------ models and connection tests

#[tokio::test]
async fn only_listed_or_verified_models_can_be_used() {
    let h = harness();
    h.svc.save_credential("fake", Secret::new(KEY.into()), None).unwrap();
    let c = h.svc.create_conversation(None, None).unwrap();
    // Before the list has been fetched, even a real model is not yet allowed.
    let e = h.svc.send_message(input(&c, "hi", "model-a"), Arc::new(Events::default())).unwrap_err();
    assert!(e.to_string().contains("not in the provider's list"), "{e}");
    let list = h.svc.list_models("fake").await.unwrap();
    assert!(list.from_provider && list.models.iter().all(|m| m.source == ModelSource::Listed));
    assert_eq!(list.models.len(), 2);
    let events = Arc::new(Events::default());
    assert!(h.svc.send_message(input(&c, "hi", "model-a"), events.clone()).is_ok());
    events.wait_finished().await;
    // An invented name is refused, and so is a default model that was never verified.
    let e = h.svc.send_message(input(&c, "again", "made-up-model"), Arc::new(Events::default())).unwrap_err();
    assert!(e.to_string().contains("not in the provider's list"), "{e}");
    let mut cfg = ProviderConfig::new("fake");
    cfg.default_model = Some("made-up-model".into());
    assert!(h.svc.update_config(cfg.clone()).is_err());
    cfg.default_model = Some("model-b".into());
    assert!(h.svc.update_config(cfg).is_ok());
    // The list is cached: asking again does not hit the provider again.
    let calls = h.fake.list_calls.load(Ordering::SeqCst);
    h.svc.list_models("fake").await.unwrap();
    assert_eq!(h.fake.list_calls.load(Ordering::SeqCst), calls);
}

#[tokio::test]
async fn a_provider_without_a_model_list_requires_typing_and_verifying_a_model() {
    let h = harness_with(false, vec![]);
    h.svc.save_credential("fake", Secret::new(KEY.into()), None).unwrap();
    let list = h.svc.list_models("fake").await.unwrap();
    assert!(!list.from_provider && list.models.is_empty());
    assert!(list.note.contains("Type the model name"));
    let c = h.svc.create_conversation(None, None).unwrap();
    assert!(h.svc.send_message(input(&c, "hi", "typed-model"), Arc::new(Events::default())).is_err(), "an unverified typed model cannot be used");
    let r = h.svc.test_connection("fake", Some("typed-model".into())).await.unwrap();
    assert!(r.ok && r.verified_model.as_deref() == Some("typed-model"));
    let list = h.svc.list_models("fake").await.unwrap();
    assert_eq!(list.models.len(), 1);
    assert_eq!(list.models[0].source, ModelSource::Verified);
    let events = Arc::new(Events::default());
    assert!(h.svc.send_message(input(&c, "hi", "typed-model"), events.clone()).is_ok());
    assert_eq!(events.wait_finished().await.kind_status(), MessageStatus::Complete);
}

trait Status {
    fn kind_status(&self) -> MessageStatus;
}
impl Status for AiUiEvent {
    fn kind_status(&self) -> MessageStatus {
        match &self.kind {
            AiUiEventKind::Finished { status, .. } => *status,
            _ => panic!("not a finish event"),
        }
    }
}

#[tokio::test]
async fn the_connection_test_reports_only_what_was_proven() {
    let h = harness();
    h.svc.save_credential("fake", Secret::new(KEY.into()), None).unwrap();
    // Key accepted but no model chosen: honest, not "connected".
    let r = h.svc.test_connection("fake", None).await.unwrap();
    assert_eq!(r.state, ConnectionState::KeyAccepted);
    assert!(!r.ok, "a key check alone is not a verified connection");
    assert!(r.steps.iter().any(|s| s.name.contains("Model not yet verified") && !s.ok));
    assert_eq!(r.models_listed, Some(2));
    // With a model, a real request is made.
    let r = h.svc.test_connection("fake", Some("model-a".into())).await.unwrap();
    assert!(r.ok && r.state == ConnectionState::Verified && r.verified_model.as_deref() == Some("model-a"));
    let sent = h.fake.requests.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].params.max_output_tokens, Some(16), "the test request is minimal");
    assert!(h.fake.keys_seen.lock().unwrap().iter().all(|k| k == KEY));
    let v = &h.svc.overview().unwrap().providers[0];
    assert_eq!(v.status.state, ConnectionState::Verified);
    assert_eq!(v.status.verified_models, vec!["model-a"]);
    // A rejected key is a failure with a classified error, and clears nothing else.
    *h.fake.auth.lock().unwrap() = Some(AiError::new(AiErrorKind::Authentication, &format!("bad key {KEY}")).status(401));
    let r = h.svc.test_connection("fake", Some("model-a".into())).await.unwrap();
    assert_eq!((r.ok, r.state), (false, ConnectionState::Failed));
    assert_eq!(r.error.as_ref().unwrap().kind, AiErrorKind::Authentication);
    assert!(!serde_json::to_string(&r).unwrap().contains(KEY), "the error text is scrubbed");
    assert!(!r.steps[0].ok);
    assert_eq!(h.fake.requests.lock().unwrap().len(), 1, "no model request is made with a key that was rejected");
    // A model the provider refuses is a failure of that step, and it does not become usable.
    *h.fake.auth.lock().unwrap() = None;
    h.fake.push(Script::Fail(AiError::new(AiErrorKind::ModelNotFound, "no such model")));
    let r = h.svc.test_connection("fake", Some("model-b".into())).await.unwrap();
    assert_eq!(r.error.as_ref().unwrap().kind, AiErrorKind::ModelNotFound);
    assert!(!h.svc.overview().unwrap().providers[0].status.verified_models.contains(&"model-b".to_string()));
}

#[tokio::test]
async fn testing_without_a_key_fails_clearly() {
    let h = harness();
    let e = h.svc.test_connection("fake", None).await.unwrap_err();
    assert!(e.to_string().contains("no API key"));
}

// ------------------------------------------------------------------ conversations and streaming

#[tokio::test]
async fn a_streamed_conversation_is_delivered_in_order_and_persisted_with_usage() {
    let h = harness();
    let c = ready(&h).await;
    h.fake.push(Script::Reply { chunks: vec!["Hel", "lo ", "there"], usage: Some(Usage { input_tokens: Some(9), output_tokens: Some(3), total_tokens: Some(12), cost_usd: None }), finish: FinishReason::Stop });
    let events = Arc::new(Events::default());
    let ack = h.svc.send_message(input(&c, "  Explain   nmap -F  ", "model-a"), events.clone()).unwrap();
    assert_eq!(h.svc.active_requests(), 1);
    let done = events.wait_finished().await;
    let all = events.all();
    assert!(matches!(all[0].kind, AiUiEventKind::Started { ref provider, ref model } if provider == "fake" && model == "model-a"));
    let text: String = all.iter().filter_map(|e| if let AiUiEventKind::Delta { text } = &e.kind { Some(text.as_str()) } else { None }).collect();
    assert_eq!(text, "Hello there");
    assert!(all.iter().all(|e| e.request_id == ack.request_id && e.conversation_id == c.id && e.message_id == ack.assistant_message_id));
    let AiUiEventKind::Finished { status, finish_reason, usage, error } = done.kind else { panic!() };
    assert_eq!((status, finish_reason, error), (MessageStatus::Complete, Some(FinishReason::Stop), None));
    assert_eq!(usage.total_tokens, Some(12));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(h.svc.active_requests(), 0);
    let d = h.svc.get_conversation(&c.id).unwrap();
    assert_eq!(d.messages.len(), 2);
    assert_eq!((d.messages[0].role, d.messages[0].content.as_str()), (Role::User, "  Explain   nmap -F  "));
    assert_eq!((d.messages[1].role, d.messages[1].content.as_str(), d.messages[1].status), (Role::Assistant, "Hello there", MessageStatus::Complete));
    assert_eq!(d.messages[1].usage.as_ref().unwrap().total_tokens, Some(12));
    assert_eq!(d.messages[1].provider.as_deref(), Some("fake"));
    assert_eq!(d.conversation.title, "Explain nmap -F", "the first message names the conversation");
    // What was sent: the safe system prompt, the user's words, no tools, no key, no other context.
    let sent = h.fake.requests.lock().unwrap().last().cloned().unwrap();
    assert!(sent.system.as_deref().unwrap().contains("cannot run commands"));
    assert!(sent.tools.is_empty(), "a model is never offered a way to run anything");
    assert_eq!(sent.messages.len(), 1);
    assert!(!format!("{sent:?}").contains(KEY));
}

#[tokio::test]
async fn follow_up_messages_carry_the_earlier_turns_and_failed_turns_are_left_out() {
    let h = harness();
    let c = ready(&h).await;
    for (text, reply) in [("first", "one"), ("second", "two")] {
        h.fake.push(Fake::reply(reply));
        let ev = Arc::new(Events::default());
        h.svc.send_message(input(&c, text, "model-a"), ev.clone()).unwrap();
        ev.wait_finished().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    h.fake.push(Script::Fail(AiError::new(AiErrorKind::InvalidRequest, "nope")));
    let ev = Arc::new(Events::default());
    h.svc.send_message(input(&c, "third", "model-a"), ev.clone()).unwrap();
    assert_eq!(ev.wait_finished().await.kind_status(), MessageStatus::Failed);
    tokio::time::sleep(Duration::from_millis(20)).await;
    h.fake.push(Fake::reply("four"));
    let ev = Arc::new(Events::default());
    h.svc.send_message(input(&c, "fourth", "model-a"), ev.clone()).unwrap();
    ev.wait_finished().await;
    let sent = h.fake.requests.lock().unwrap().last().cloned().unwrap();
    let roles: Vec<_> = sent.messages.iter().map(|m| (m.role, m.content.as_str())).collect();
    assert_eq!(roles, vec![(Role::User, "first"), (Role::Assistant, "one"), (Role::User, "second"), (Role::Assistant, "two"), (Role::User, "third\n\nfourth")], "a user turn whose answer failed is merged with the next one so roles alternate");
}

#[tokio::test]
async fn transient_errors_before_any_text_are_retried_a_bounded_number_of_times() {
    let h = harness();
    let c = ready(&h).await;
    h.fake.push(Script::Fail(AiError::new(AiErrorKind::RateLimit, "slow down").retry_after(Some(0))));
    h.fake.push(Script::Fail(AiError::new(AiErrorKind::Overloaded, "busy")));
    h.fake.push(Fake::reply("finally"));
    let ev = Arc::new(Events::default());
    h.svc.send_message(input(&c, "hi", "model-a"), ev.clone()).unwrap();
    let done = ev.wait_finished().await;
    assert_eq!(done.kind_status(), MessageStatus::Complete);
    assert_eq!(h.fake.requests.lock().unwrap().len(), 3);
    assert_eq!(ev.all().iter().filter(|e| matches!(e.kind, AiUiEventKind::Retrying { .. })).count(), 2, "the interface is told about each wait");
    // Giving up after the limit shows the last error; the same model and provider were used throughout.
    let before = h.fake.requests.lock().unwrap().len();
    for _ in 0..5 {
        h.fake.push(Script::Fail(AiError::new(AiErrorKind::Server, "down")));
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    let ev = Arc::new(Events::default());
    h.svc.send_message(input(&c, "again", "model-a"), ev.clone()).unwrap();
    let AiUiEventKind::Finished { status, error, .. } = ev.wait_finished().await.kind else { panic!() };
    assert_eq!(status, MessageStatus::Failed);
    assert_eq!(error.unwrap().kind, AiErrorKind::Server);
    assert_eq!(h.fake.requests.lock().unwrap().len() - before, 3, "three attempts, no more");
    assert!(h.fake.requests.lock().unwrap().iter().all(|r| r.model == "model-a"));
}

#[tokio::test]
async fn permanent_errors_and_errors_after_text_are_never_retried() {
    let h = harness();
    let c = ready(&h).await;
    for kind in [AiErrorKind::Authentication, AiErrorKind::Quota, AiErrorKind::ModelNotFound, AiErrorKind::InvalidRequest, AiErrorKind::ContentFiltered] {
        let before = h.fake.requests.lock().unwrap().len();
        h.fake.push(Script::Fail(AiError::new(kind, "no")));
        let ev = Arc::new(Events::default());
        h.svc.send_message(input(&c, "hi", "model-a"), ev.clone()).unwrap();
        ev.wait_finished().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(h.fake.requests.lock().unwrap().len() - before, 1, "{kind:?} is not retried");
    }
    // The connection breaks after text arrived: no automatic retry (it would repeat or duplicate text); the partial text is kept.
    let before = h.fake.requests.lock().unwrap().len();
    h.fake.push(Script::FailAfterText { chunks: vec!["Part", "ial"], error: AiError::new(AiErrorKind::Network, "connection reset") });
    let ev = Arc::new(Events::default());
    let ack = h.svc.send_message(input(&c, "long answer please", "model-a"), ev.clone()).unwrap();
    let AiUiEventKind::Finished { status, error, .. } = ev.wait_finished().await.kind else { panic!() };
    assert_eq!(status, MessageStatus::Interrupted);
    assert_eq!(error.unwrap().kind, AiErrorKind::Network);
    assert_eq!(h.fake.requests.lock().unwrap().len() - before, 1);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let m = h.svc.get_conversation(&c.id).unwrap().messages.into_iter().find(|m| m.id == ack.assistant_message_id).unwrap();
    assert_eq!((m.content.as_str(), m.status), ("Partial", MessageStatus::Interrupted));
    assert_eq!(m.error_code.as_deref(), Some("ai_network"));
}

#[tokio::test]
async fn stopping_a_response_keeps_what_was_shown_and_only_a_user_action_retries() {
    let h = harness();
    let c = ready(&h).await;
    h.fake.push(Script::Hang { chunks: vec!["Start of an answer"] });
    let ev = Arc::new(Events::default());
    let ack = h.svc.send_message(input(&c, "write a lot", "model-a"), ev.clone()).unwrap();
    for _ in 0..200 {
        if ev.all().iter().any(|e| matches!(e.kind, AiUiEventKind::Delta { .. })) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(matches!(h.svc.send_message(input(&c, "second at once", "model-a"), Arc::new(Events::default())), Err(AppError::Busy(_))), "one response at a time per conversation");
    assert!(matches!(h.svc.delete_conversation(&c.id), Err(AppError::Busy(_))), "a conversation being written cannot be deleted");
    h.svc.cancel(&ack.request_id).unwrap();
    let AiUiEventKind::Finished { status, error, .. } = ev.wait_finished().await.kind else { panic!() };
    assert_eq!(status, MessageStatus::Cancelled);
    assert_eq!(error.unwrap().kind, AiErrorKind::Cancelled);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(h.fake.requests.lock().unwrap().len(), 1, "nothing is re-sent by itself");
    let m = h.svc.get_conversation(&c.id).unwrap().messages.pop().unwrap();
    assert_eq!((m.content.as_str(), m.status), ("Start of an answer", MessageStatus::Cancelled));
    assert!(h.svc.cancel(&ack.request_id).is_err(), "a finished request cannot be cancelled");
    // Retry is explicit: the stopped answer is replaced by a new one for the same question.
    h.fake.push(Fake::reply("second try"));
    let ev2 = Arc::new(Events::default());
    h.svc.retry_last(&c.id, "fake".into(), "model-a".into(), ev2.clone()).unwrap();
    assert_eq!(ev2.wait_finished().await.kind_status(), MessageStatus::Complete);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let msgs = h.svc.get_conversation(&c.id).unwrap().messages;
    assert_eq!(msgs.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["write a lot", "second try"]);
    assert!(h.svc.retry_last(&c.id, "fake".into(), "model-a".into(), Arc::new(Events::default())).is_err(), "a finished answer cannot be 'retried'");
}

#[tokio::test]
async fn errors_that_echo_the_key_are_scrubbed_before_they_are_shown_or_stored() {
    let h = harness();
    let c = ready(&h).await;
    h.fake.push(Script::Fail(AiError { kind: AiErrorKind::Authentication, message: format!("bad credentials {KEY} for project"), status: Some(401), retry_after_secs: None }));
    let ev = Arc::new(Events::default());
    let ack = h.svc.send_message(input(&c, "hi", "model-a"), ev.clone()).unwrap();
    ev.wait_finished().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let shown = serde_json::to_string(&ev.all()).unwrap();
    assert!(!shown.contains(KEY) && !shown.contains("0123456789abcdef"), "{shown}");
    let stored = h.svc.get_conversation(&c.id).unwrap().messages.into_iter().find(|m| m.id == ack.assistant_message_id).unwrap();
    assert!(!stored.error_message.unwrap().contains("0123456789abcdef"));
    assert!(!serde_json::to_string(&h.svc.get_conversation(&c.id).unwrap()).unwrap().contains(KEY));
}

#[tokio::test]
async fn messages_that_contain_credentials_or_are_too_large_are_refused() {
    let h = harness();
    let c = ready(&h).await;
    for bad in [KEY.to_string(), "here is my key sk-proj-abcdefghijklmnopqrstuvwxyz0123".to_string(), "AIzaSyA1234567890abcdefghijklmnopqrstuv".to_string(), "".to_string(), "   \n ".to_string()] {
        assert!(h.svc.send_message(input(&c, &bad, "model-a"), Arc::new(Events::default())).is_err(), "{bad:?}");
    }
    let e = h.svc.send_message(input(&c, &"x".repeat(MAX_USER_MESSAGE_CHARS + 1), "model-a"), Arc::new(Events::default())).unwrap_err();
    assert!(matches!(&e, AppError::Ai(a) if a.kind == AiErrorKind::TooLarge));
    assert_eq!(h.fake.requests.lock().unwrap().len(), 0, "nothing reached the provider");
    assert!(h.svc.get_conversation(&c.id).unwrap().messages.is_empty(), "and nothing was stored");
    assert!(h.svc.send_message(input(&c, "what is a bearer token?", "model-a"), Arc::new(Events::default())).is_ok());
}

#[tokio::test]
async fn the_conversation_sent_to_the_provider_is_bounded() {
    let h = harness();
    h.svc.update_settings(AiSettings { confirm_large_chars: 0, ..Default::default() }).unwrap();
    let c = ready(&h).await;
    let long = "w".repeat(45_000);
    for i in 0..6 {
        h.fake.push(Fake::reply("ack"));
        let ev = Arc::new(Events::default());
        h.svc.send_message(input(&c, &format!("{i} {long}"), "model-a"), ev.clone()).unwrap();
        ev.wait_finished().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let sent = h.fake.requests.lock().unwrap().last().cloned().unwrap();
    let total: usize = sent.messages.iter().map(|m| m.content.chars().count()).sum();
    assert!(total <= MAX_CONTEXT_CHARS, "{total}");
    assert_eq!(sent.messages.first().unwrap().role, Role::User);
    assert!(sent.messages.last().unwrap().content.starts_with("5 "), "the newest message is always kept");
}

// ------------------------------------------------------------------ persistence and retention

#[tokio::test]
async fn conversations_can_be_listed_renamed_by_first_message_and_deleted() {
    let h = harness();
    let a = ready(&h).await;
    let b = h.svc.create_conversation(None, None).unwrap();
    let ev = Arc::new(Events::default());
    h.svc.send_message(input(&b, "topic b", "model-a"), ev.clone()).unwrap();
    ev.wait_finished().await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let list = h.svc.list_conversations().unwrap();
    assert_eq!(list.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().len(), 2);
    assert_eq!(list[0].id, b.id, "most recently used first");
    h.svc.delete_conversation(&a.id).unwrap();
    assert_eq!(h.svc.list_conversations().unwrap().len(), 1);
    assert_eq!(h.svc.delete_all_conversations().unwrap(), 1);
    assert!(h.svc.get_conversation(&b.id).is_err());
}

#[test]
fn retention_deletes_old_conversations_and_zero_days_deletes_everything_and_startup_closes_cut_off_replies() {
    let h = harness();
    let old = Conversation { id: "old".into(), title: "old".into(), provider: None, model: None, created_at: "2020-01-01T00:00:00Z".into(), updated_at: "2020-01-01T00:00:00Z".into() };
    h.store.create_conversation(&old).unwrap();
    let fresh = h.svc.create_conversation(None, None).unwrap();
    h.store
        .append_message(&StoredMessage { id: "m".into(), conversation_id: fresh.id.clone(), seq: 0, role: Role::Assistant, content: "half".into(), status: MessageStatus::Streaming, error_code: None, error_message: None, provider: None, model: None, usage: None, created_at: "2026-01-01T00:00:00Z".into() })
        .unwrap();
    let (closed, purged) = h.svc.startup().unwrap();
    assert_eq!((closed, purged), (1, 1), "the old conversation went, the cut-off reply was closed");
    assert_eq!(h.svc.get_conversation(&fresh.id).unwrap().messages[0].status, MessageStatus::Interrupted);
    assert!(h.svc.update_settings(AiSettings { history_days: 4000, ..Default::default() }).is_err());
    h.svc.update_settings(AiSettings { history_days: 0, ..Default::default() }).unwrap();
    assert!(h.svc.list_conversations().unwrap().is_empty(), "0 days keeps nothing");
    assert_eq!(h.svc.overview().unwrap().settings.history_days, 0);
}

// ------------------------------------------------------------------ provider switching never happens silently

#[tokio::test]
async fn a_failure_is_never_answered_by_another_provider_or_model() {
    let h = harness();
    let c = ready(&h).await;
    h.svc.save_credential("other", Secret::new("sk-test-OTHER-0123456789".into()), None).unwrap();
    h.svc.list_models("other").await.unwrap();
    h.fake.push(Script::Fail(AiError::new(AiErrorKind::Server, "down")));
    h.fake.push(Script::Fail(AiError::new(AiErrorKind::Server, "down")));
    h.fake.push(Script::Fail(AiError::new(AiErrorKind::Server, "down")));
    let ev = Arc::new(Events::default());
    h.svc.send_message(input(&c, "hi", "model-a"), ev.clone()).unwrap();
    assert_eq!(ev.wait_finished().await.kind_status(), MessageStatus::Failed);
    let d = h.svc.get_conversation(&c.id).unwrap();
    assert_eq!(d.conversation.provider.as_deref(), Some("fake"));
    assert_eq!(h.svc.overview().unwrap().providers.iter().find(|p| p.active).unwrap().descriptor.id, "fake", "the active provider did not change");
    assert!(h.svc.set_active("other").is_ok(), "switching is an explicit user action");
}

#[test]
fn providers_can_be_registered_later_without_touching_the_service() {
    let mut reg = ProviderRegistry::builtin();
    let ids: Vec<String> = reg.descriptors().into_iter().map(|d| d.id).collect();
    assert_eq!(ids, ["anthropic", "gemini", "openai", "openrouter"]);
    reg.register(Fake::new("my-gateway", true, vec!["m"])).unwrap();
    assert!(reg.register(Fake::new("my-gateway", true, vec![])).is_err(), "ids are unique");
    assert!(reg.register(Fake::new("Bad Id!", true, vec![])).is_err());
    let svc = AiService::new(Arc::new(reg), Arc::new(MemoryCredentialStore::default()), Arc::new(SqliteAiStore::in_memory().unwrap()));
    let overview = svc.overview().unwrap();
    assert_eq!(overview.providers.len(), 5);
    assert!(overview.providers.iter().all(|p| !p.has_key && !p.active && p.status.state == ConnectionState::Untested));
    assert!(overview.credential_backend.contains("test"));
    for p in &overview.providers {
        assert!(p.descriptor.key_page.starts_with("https://") && p.descriptor.docs_page.starts_with("https://"), "{}", p.descriptor.id);
        assert!(!p.descriptor.verification_note.is_empty() && !p.descriptor.billing_note.is_empty());
    }
}

#[test]
fn the_backoff_grows_respects_retry_after_and_is_capped() {
    let b = Backoff { base_ms: 1000 };
    let w1 = b.wait(1, None);
    let w2 = b.wait(2, None);
    let w3 = b.wait(3, None);
    assert!(w1 >= Duration::from_millis(1000) && w1 < Duration::from_millis(1300));
    assert!(w2 >= Duration::from_millis(2000) && w3 >= Duration::from_millis(4000));
    assert!(b.wait(1, Some(9)) >= Duration::from_secs(9));
    assert!(b.wait(1, Some(3000)) <= Duration::from_secs(20), "never waits longer than the cap");
    assert!(b.wait(30, None) <= Duration::from_secs(20));
}
