//! Provider-independent model for talking to hosted AI models: messages, requests, stream events, usage, capabilities,
//! configuration (never secrets) and a classified error type. Nothing here performs I/O.

use super::secret::Secret;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

// ------------------------------------------------------------------ limits

pub const MAX_USER_MESSAGE_CHARS: usize = 50_000;
/// Upper bound on the conversation text sent with one request (older messages are dropped first).
pub const MAX_CONTEXT_CHARS: usize = 200_000;
pub const MAX_RESPONSE_CHARS: usize = 200_000;
pub const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;
pub const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;
pub const MAX_ERROR_MESSAGE_CHARS: usize = 400;
pub const MIN_TIMEOUT_SECS: u32 = 5;
pub const MAX_TIMEOUT_SECS: u32 = 600;
pub const DEFAULT_TIMEOUT_SECS: u32 = 60;
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 2048;
pub const MAX_OUTPUT_TOKENS_LIMIT: u32 = 65_536;

// ------------------------------------------------------------------ messages

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON text of the arguments, as produced by the model (not validated or executed by Vixalune).
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: text.into(), tool_calls: vec![], tool_call_id: None }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: text.into(), tool_calls: vec![], tool_call_id: None }
    }
}

/// Describes a function the model may ask to call. The assistant in Vixalune offers none: a model's reply can never run
/// anything on this computer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationParams {
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDescription>,
    pub params: GenerationParams,
    /// Longest silence tolerated while waiting for the next piece of the response.
    pub idle_timeout_secs: u32,
}

// ------------------------------------------------------------------ results

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    /// Cost in US dollars **as reported by the provider**. `None` when the provider does not report one; Vixalune never
    /// estimates a price.
    pub cost_usd: Option<f64>,
}

impl Usage {
    pub fn is_empty(&self) -> bool {
        self.input_tokens.is_none() && self.output_tokens.is_none() && self.total_tokens.is_none() && self.cost_usd.is_none()
    }

    /// Later reports replace earlier ones field by field (providers send input tokens first and output tokens last).
    pub fn merge(&mut self, later: &Usage) {
        self.input_tokens = later.input_tokens.or(self.input_tokens);
        self.output_tokens = later.output_tokens.or(self.output_tokens);
        self.total_tokens = later.total_tokens.or(self.total_tokens);
        self.cost_usd = later.cost_usd.or(self.cost_usd);
        if self.total_tokens.is_none() {
            if let (Some(i), Some(o)) = (self.input_tokens, self.output_tokens) {
                self.total_tokens = Some(i + o);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    /// The model hit its output limit (or Vixalune's response size limit).
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Started { model: Option<String> },
    TextDelta { text: String },
    ToolCallStart { index: usize, id: String, name: String },
    ToolCallArguments { index: usize, fragment: String },
    Usage { usage: Usage },
    Finished { reason: FinishReason },
}

/// Everything a finished (or ended) request produced, assembled from the stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatOutcome {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish: Option<FinishReason>,
    pub model: Option<String>,
    pub truncated_by_limit: bool,
}

impl ChatOutcome {
    /// Fold one event into the outcome. Returns `false` once the response size limit is reached (the caller stops reading).
    pub fn apply(&mut self, ev: &StreamEvent) -> bool {
        match ev {
            StreamEvent::Started { model } => self.model = model.clone().or(self.model.take()),
            StreamEvent::TextDelta { text } => {
                if self.text.chars().count() + text.chars().count() > MAX_RESPONSE_CHARS {
                    let room = MAX_RESPONSE_CHARS.saturating_sub(self.text.chars().count());
                    self.text.extend(text.chars().take(room));
                    self.truncated_by_limit = true;
                    return false;
                }
                self.text.push_str(text);
            }
            StreamEvent::ToolCallStart { index, id, name } => {
                while self.tool_calls.len() <= *index {
                    self.tool_calls.push(ToolCall { id: String::new(), name: String::new(), arguments: String::new() });
                }
                self.tool_calls[*index].id = id.clone();
                self.tool_calls[*index].name = name.clone();
            }
            StreamEvent::ToolCallArguments { index, fragment } => {
                while self.tool_calls.len() <= *index {
                    self.tool_calls.push(ToolCall { id: String::new(), name: String::new(), arguments: String::new() });
                }
                if self.tool_calls[*index].arguments.len() + fragment.len() <= 64 * 1024 {
                    self.tool_calls[*index].arguments.push_str(fragment);
                }
            }
            StreamEvent::Usage { usage } => self.usage.merge(usage),
            StreamEvent::Finished { reason } => self.finish = Some(reason.clone()),
        }
        true
    }
}

// ------------------------------------------------------------------ errors

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiErrorKind {
    /// The provider rejected the key (missing, wrong, revoked).
    Authentication,
    /// The key is valid but not allowed to do this (permissions, region, model access).
    Permission,
    /// Credit, billing or plan limit reached.
    Quota,
    /// Too many requests; try again later.
    RateLimit,
    Network,
    Timeout,
    ModelNotFound,
    InvalidRequest,
    ContentFiltered,
    Server,
    Overloaded,
    Cancelled,
    Unsupported,
    Configuration,
    CredentialStore,
    Protocol,
    TooLarge,
}

impl AiErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Authentication => "ai_authentication",
            Self::Permission => "ai_permission",
            Self::Quota => "ai_quota",
            Self::RateLimit => "ai_rate_limit",
            Self::Network => "ai_network",
            Self::Timeout => "ai_timeout",
            Self::ModelNotFound => "ai_model",
            Self::InvalidRequest => "ai_invalid_request",
            Self::ContentFiltered => "ai_content_filtered",
            Self::Server => "ai_server",
            Self::Overloaded => "ai_overloaded",
            Self::Cancelled => "ai_cancelled",
            Self::Unsupported => "ai_unsupported",
            Self::Configuration => "ai_configuration",
            Self::CredentialStore => "ai_credential_store",
            Self::Protocol => "ai_protocol",
            Self::TooLarge => "ai_too_large",
        }
    }

    /// What the user should do next. Never empty.
    pub fn recovery(self) -> &'static str {
        match self {
            Self::Authentication => "Check the API key in Settings, AI Providers: replace it with a valid key for this provider and test the connection.",
            Self::Permission => "The key is valid but not allowed to do this. Check the key's permissions, the project it belongs to, and whether your account can use this model.",
            Self::Quota => "Your account has no remaining credit or has reached a plan limit. Check billing with the provider, then try again.",
            Self::RateLimit => "The provider is limiting requests. Wait a little and try again; Vixalune retries a few times by itself before a response starts.",
            Self::Network => "Check your internet connection, proxy and firewall, then try again. Nothing was sent if the connection was never made.",
            Self::Timeout => "The provider did not answer in time. Try again, or raise the request timeout in Settings, AI Providers.",
            Self::ModelNotFound => "Choose a model from the list, or verify the model name with Test connection. The provider does not know this model for your key.",
            Self::InvalidRequest => "The provider rejected the request. Shorten the message or change the generation settings and try again.",
            Self::ContentFiltered => "The provider declined to answer this request. Rephrase it and try again.",
            Self::Server => "The provider had a problem. Try again in a moment, or check the provider's status page. Vixalune does not switch to another provider by itself.",
            Self::Overloaded => "The provider is overloaded. Try again in a moment. Vixalune does not switch to another provider or model by itself.",
            Self::Cancelled => "You stopped the response. Send the message again when ready.",
            Self::Unsupported => "This provider or model does not support that. Choose another model or provider.",
            Self::Configuration => "Open Settings, AI Providers and complete the configuration (key, model).",
            Self::CredentialStore => "Windows Credential Manager could not be used, so the key was not saved or read. Vixalune does not store keys anywhere else. Check that your Windows account can use Credential Manager, then try again.",
            Self::Protocol => "The provider sent something unexpected. Try again; if it keeps happening, the provider may have changed its API.",
            Self::TooLarge => "The message or response was larger than Vixalune allows. Shorten the message or start a new conversation.",
        }
    }

    /// Worth retrying automatically, but only before any part of a response has been received.
    pub fn retryable(self) -> bool {
        matches!(self, Self::RateLimit | Self::Network | Self::Timeout | Self::Server | Self::Overloaded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiError {
    pub kind: AiErrorKind,
    /// Already redacted: contains no key, no bearer token and no URL credentials.
    pub message: String,
    pub status: Option<u16>,
    pub retry_after_secs: Option<u64>,
}

impl AiError {
    pub fn new(kind: AiErrorKind, message: impl AsRef<str>) -> Self {
        Self { kind, message: sanitize_message(message.as_ref(), None), status: None, retry_after_secs: None }
    }

    pub fn with_secret(kind: AiErrorKind, message: impl AsRef<str>, secret: &Secret) -> Self {
        Self { kind, message: sanitize_message(message.as_ref(), Some(secret.expose())), status: None, retry_after_secs: None }
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn retry_after(mut self, secs: Option<u64>) -> Self {
        self.retry_after_secs = secs;
        self
    }

    pub fn cancelled() -> Self {
        Self::new(AiErrorKind::Cancelled, "the request was cancelled")
    }

    pub fn retryable(&self) -> bool {
        self.kind.retryable()
    }

    /// A copy of this error with any occurrence of `secret` removed (used when an error crosses a layer that knows the key).
    pub fn scrubbed(mut self, secret: &Secret) -> Self {
        self.message = sanitize_message(&self.message, Some(secret.expose()));
        self
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AiError {}

pub type AiResult<T> = Result<T, AiError>;

// ------------------------------------------------------------------ redaction

fn patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            r"sk-ant-[A-Za-z0-9_\-]{6,}",
            r"sk-or-[A-Za-z0-9_\-]{6,}",
            r"sk-[A-Za-z0-9_\-]{12,}",
            r"AIza[0-9A-Za-z_\-]{16,}",
            r"(?i)bearer\s+[A-Za-z0-9._~+/=\-]{6,}",
            r"(?i)(x-api-key|x-goog-api-key|api[-_]?key|authorization)\s*[:=]\s*\S+",
            r"(?i)([?&](key|api_key|apikey|access_token)=)[^&\s]+",
            r"(?i)(https?://)[^\s/:@]+:[^\s/@]+@",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static pattern"))
        .collect()
    })
}

/// Make text safe to show, store or log: the exact secret (if known) and anything shaped like a credential are masked,
/// control characters are removed and the length is bounded.
pub fn sanitize_message(text: &str, secret: Option<&str>) -> String {
    let mut s: String = text.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect();
    if let Some(k) = secret {
        if k.len() >= 4 {
            s = s.replace(k, "[redacted]");
            // A key echoed back in pieces (for example the first and last characters) is not a practical leak, but a
            // URL-encoded copy is.
            let enc: String = k.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
            if enc != k {
                s = s.replace(&enc, "[redacted]");
            }
        }
    }
    for re in patterns() {
        s = re.replace_all(&s, |caps: &regex::Captures| match caps.get(1) {
            Some(prefix) if re.as_str().starts_with("(?i)([?&]") || re.as_str().starts_with("(?i)(https?") => format!("{}[redacted]", prefix.as_str()),
            Some(name) if re.as_str().starts_with("(?i)(x-api-key") => format!("{}: [redacted]", name.as_str()),
            _ => "[redacted]".to_string(),
        })
        .into_owned();
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > MAX_ERROR_MESSAGE_CHARS {
        let cut: String = s.chars().take(MAX_ERROR_MESSAGE_CHARS).collect();
        format!("{cut}...")
    } else {
        s
    }
}

/// Text that looks like it contains a credential, used to stop a user pasting a key into a message that would be sent to a
/// provider and stored in the history. Deliberately narrow (key-shaped strings, private keys, credentials in URLs) so ordinary
/// talk about "bearer tokens" or "API keys" is not refused.
pub fn looks_like_credential(text: &str) -> bool {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    let pats = P.get_or_init(|| {
        [
            r"\bsk-ant-[A-Za-z0-9_\-]{20,}",
            r"\bsk-or-[A-Za-z0-9_\-]{20,}",
            r"\bsk-[A-Za-z0-9_\-]{24,}",
            r"\bAIza[0-9A-Za-z_\-]{30,}",
            r"(?i)authorization\s*:\s*bearer\s+[A-Za-z0-9._~+/=\-]{20,}",
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
            r"(?i)https?://[^\s/:@]+:[^\s/@]{3,}@",
            r"\bAKIA[0-9A-Z]{16}\b",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static pattern"))
        .collect()
    });
    pats.iter().any(|re| re.is_match(text))
}

// ------------------------------------------------------------------ providers: metadata and configuration

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_calling: bool,
    pub structured_output: bool,
    pub model_listing: bool,
    pub usage_reporting: bool,
    /// The provider reports a price for each request (otherwise Vixalune shows tokens only).
    pub cost_reporting: bool,
    pub cancellation: bool,
    /// The provider's model list says how large each model's context is.
    pub context_limits: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub capabilities: ProviderCapabilities,
    /// Shown while typing a key, for example "starts with sk-ant-". Never validated as proof of anything.
    pub key_hint: String,
    /// Official page where a key is created (https).
    pub key_page: String,
    pub docs_page: String,
    /// What the user should know about money for this provider.
    pub billing_note: String,
    /// What this adapter has actually been verified against, in plain words.
    pub verification_note: String,
    /// Extra settings this provider understands, by name (for provider-specific configuration).
    pub extra_settings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// Returned by the provider's own model list.
    Listed,
    /// Typed by the user and then proven by a real test request.
    Verified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// US dollars per million input / output tokens, as published by the provider's model list (information only).
    pub input_per_mtok: Option<f64>,
    pub output_per_mtok: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub supports_tools: Option<bool>,
    pub pricing: Option<ModelPricing>,
    pub source: ModelSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub provider: String,
    /// A name the user gives this key ("work", "personal"); display only.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Provider-specific switches (see `ProviderDescriptor::extra_settings`), e.g. `allow_upstream_fallbacks` for OpenRouter.
    #[serde(default)]
    pub extra: std::collections::BTreeMap<String, String>,
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_SECS
}

impl ProviderConfig {
    pub fn new(provider: &str) -> Self {
        Self { provider: provider.into(), label: None, default_model: None, timeout_secs: DEFAULT_TIMEOUT_SECS, temperature: None, max_output_tokens: None, extra: Default::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Never tested since the key was saved.
    Untested,
    /// The provider accepted the key, but no model has been proven yet.
    KeyAccepted,
    /// The key was accepted and a real request with the chosen model succeeded.
    Verified,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestStep {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub state: ConnectionState,
    pub tested_at: Option<String>,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub steps: Vec<TestStep>,
    pub models_listed: Option<usize>,
    /// Models proven by a real test request (manual entries must be in here to be usable).
    pub verified_models: Vec<String>,
}

impl Default for ProviderStatus {
    fn default() -> Self {
        Self { state: ConnectionState::Untested, tested_at: None, error_code: None, message: None, steps: vec![], models_listed: None, verified_models: vec![] }
    }
}

// ------------------------------------------------------------------ validation

pub fn valid_provider_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !s.starts_with('-')
}

/// Model identifiers differ between providers but are always short, printable and free of whitespace or shell syntax.
pub fn valid_model_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && !s.starts_with(['-', '/', '.']) && !s.contains("..") && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-' | '@' | '+'))
}

pub fn validate_api_key_format(key: &str) -> AiResult<()> {
    let bad = |m: &str| Err(AiError::new(AiErrorKind::Configuration, m));
    if key.is_empty() {
        return bad("the API key is empty");
    }
    if key.len() > 512 {
        return bad("the API key is too long to be a real key");
    }
    if key.chars().any(|c| c.is_whitespace() || c.is_control() || !c.is_ascii()) {
        return bad("the API key contains spaces, line breaks or characters that cannot be in a key; paste it again without extra characters");
    }
    if key.len() < 8 {
        return bad("the API key is too short to be a real key");
    }
    Ok(())
}

/// Checks that apply to every provider. Adapters add their own (temperature ranges, extra settings).
pub fn validate_config_generic(c: &ProviderConfig) -> AiResult<()> {
    let bad = |m: String| Err(AiError::new(AiErrorKind::Configuration, m));
    if !valid_provider_id(&c.provider) {
        return bad("unknown provider".into());
    }
    if let Some(l) = &c.label {
        if l.chars().count() > 60 || l.chars().any(|ch| ch.is_control()) {
            return bad("the label is too long or contains control characters".into());
        }
    }
    if let Some(m) = &c.default_model {
        if !valid_model_id(m) {
            return bad(format!("'{}' is not a valid model name", sanitize_message(m, None)));
        }
    }
    if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&c.timeout_secs) {
        return bad(format!("the request timeout must be {MIN_TIMEOUT_SECS} to {MAX_TIMEOUT_SECS} seconds"));
    }
    if let Some(t) = c.temperature {
        if !t.is_finite() || !(0.0..=2.0).contains(&t) {
            return bad("the temperature must be between 0 and 2".into());
        }
    }
    if let Some(n) = c.max_output_tokens {
        if !(1..=MAX_OUTPUT_TOKENS_LIMIT).contains(&n) {
            return bad(format!("the maximum response length must be 1 to {MAX_OUTPUT_TOKENS_LIMIT} tokens"));
        }
    }
    if c.extra.len() > 8 || c.extra.iter().any(|(k, v)| k.len() > 40 || v.len() > 200 || v.chars().any(|ch| ch.is_control())) {
        return bad("too many or too long extra settings".into());
    }
    Ok(())
}

// ------------------------------------------------------------------ conversations (persisted, never containing keys)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Complete,
    Streaming,
    Failed,
    Cancelled,
    /// The connection broke or the app stopped after part of the response had arrived. The partial text is kept.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub conversation_id: String,
    pub seq: i64,
    pub role: Role,
    pub content: String,
    pub status: MessageStatus,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// User-chosen settings that apply to all providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSettings {
    /// Conversations not touched for this many days are deleted (0 = delete everything at the next start).
    #[serde(default = "default_history_days")]
    pub history_days: u32,
    /// The person agreed that package names and versions they ask about may be sent to public vulnerability databases (OSV, NVD).
    #[serde(default)]
    pub allow_vulnerability_lookup: bool,
}

fn default_history_days() -> u32 {
    30
}

impl Default for AiSettings {
    fn default() -> Self {
        Self { history_days: default_history_days(), allow_vulnerability_lookup: false }
    }
}

impl AiSettings {
    pub fn validate(&self) -> super::AppResult<()> {
        if self.history_days > 3650 {
            return Err(super::AppError::Validation("conversation history must be kept 0-3650 days".into()));
        }
        Ok(())
    }
}

/// What a user-visible event about a streaming request looks like. Sent to the UI as it happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiUiEvent {
    pub request_id: String,
    pub conversation_id: String,
    pub message_id: String,
    #[serde(flatten)]
    pub kind: AiUiEventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AiUiEventKind {
    Started { provider: String, model: String },
    Delta { text: String },
    /// A pause between automatic retries (before any text has arrived).
    Retrying { attempt: u32, wait_secs: u64, reason: String },
    Usage { usage: Usage },
    Finished { status: MessageStatus, finish_reason: Option<FinishReason>, usage: Usage, error: Option<AiErrorDto> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiErrorDto {
    pub code: String,
    pub kind: AiErrorKind,
    pub message: String,
    pub recovery: String,
}

impl From<&AiError> for AiErrorDto {
    fn from(e: &AiError) -> Self {
        Self { code: e.kind.code().into(), kind: e.kind, message: e.message.clone(), recovery: e.kind.recovery().into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_removes_keys_tokens_and_credentials_wherever_they_appear() {
        let key = "sk-live-0123456789abcdef";
        let samples = [
            format!("invalid key {key} for project"),
            format!("Authorization: Bearer {key}"),
            "x-api-key: sk-ant-api03-ABCDEFGHIJ".to_string(),
            "https://user:hunter2pass@example.test/path".to_string(),
            "GET /v1beta/models?key=AIzaSyA1234567890abcdefghij&x=1".to_string(),
            "token sk-or-v1-abcdef123456 rejected".to_string(),
            format!("encoded {}", key.replace('-', "%2D")),
        ];
        for s in samples {
            let out = sanitize_message(&s, Some(key));
            for leaked in [key, "hunter2pass", "ABCDEFGHIJ", "AIzaSyA1234567890abcdefghij", "abcdef123456"] {
                assert!(!out.contains(leaked), "{leaked} leaked from {s:?}: {out:?}");
            }
        }
        assert!(sanitize_message(&"x".repeat(5000), None).chars().count() <= MAX_ERROR_MESSAGE_CHARS + 3);
        assert!(!sanitize_message("a\u{0}b\r\nc", None).contains('\u{0}'));
    }

    #[test]
    fn credential_shaped_text_is_recognised_but_ordinary_talk_is_not() {
        for bad in [
            "my key is sk-proj-abcdefghijklmnopqrstuvwxyz0123",
            "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "AIzaSyA1234567890abcdefghijklmnopqrstuv",
            "Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456",
            "-----BEGIN RSA PRIVATE KEY-----",
            "https://user:hunter2@example.test/x",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            assert!(looks_like_credential(bad), "{bad}");
        }
        for ok in ["what is a bearer token?", "how do API keys work", "explain sk and pk in cryptography", "nmap -sV 127.0.0.1", "the password field is empty"] {
            assert!(!looks_like_credential(ok), "{ok}");
        }
    }

    #[test]
    fn usage_reports_merge_field_by_field_and_never_invent_a_cost() {
        let mut u = Usage { input_tokens: Some(10), ..Default::default() };
        u.merge(&Usage { output_tokens: Some(5), ..Default::default() });
        assert_eq!((u.input_tokens, u.output_tokens, u.total_tokens, u.cost_usd), (Some(10), Some(5), Some(15), None));
        u.merge(&Usage { cost_usd: Some(0.002), ..Default::default() });
        assert_eq!(u.cost_usd, Some(0.002));
    }

    #[test]
    fn the_outcome_stops_at_the_response_size_limit() {
        let mut o = ChatOutcome::default();
        assert!(o.apply(&StreamEvent::TextDelta { text: "x".repeat(MAX_RESPONSE_CHARS - 2) }));
        assert!(!o.apply(&StreamEvent::TextDelta { text: "abcdef".into() }));
        assert_eq!(o.text.chars().count(), MAX_RESPONSE_CHARS);
        assert!(o.truncated_by_limit);
    }

    #[test]
    fn tool_calls_assemble_from_fragments() {
        let mut o = ChatOutcome::default();
        o.apply(&StreamEvent::ToolCallStart { index: 0, id: "c1".into(), name: "lookup".into() });
        o.apply(&StreamEvent::ToolCallArguments { index: 0, fragment: "{\"a\":".into() });
        o.apply(&StreamEvent::ToolCallArguments { index: 0, fragment: "1}".into() });
        assert_eq!(o.tool_calls, vec![ToolCall { id: "c1".into(), name: "lookup".into(), arguments: "{\"a\":1}".into() }]);
    }

    #[test]
    fn keys_and_models_are_validated_without_echoing_the_key() {
        for bad in ["", "short", "has space in it 123", "line\nbreak1234", "ünïcödé-key-12345", &"k".repeat(600)] {
            let e = validate_api_key_format(bad).unwrap_err();
            assert_eq!(e.kind, AiErrorKind::Configuration);
            assert!(!e.message.contains(bad) || bad.is_empty() || bad == "short", "{}", e.message);
        }
        assert!(validate_api_key_format("sk-proj-abcdefgh12345678").is_ok());
        for ok in ["gpt-x", "claude-y", "gemini-z:latest", "openai/gpt-4o", "meta-llama/llama-3.1-8b-instruct:free", "models/x"] {
            assert!(valid_model_id(ok), "{ok}");
        }
        for bad in ["", "a b", "a;b", "../x", "-x", "x\ny", "a$(b)", &"m".repeat(201)] {
            assert!(!valid_model_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn generic_config_checks_ranges() {
        let mut c = ProviderConfig::new("openai");
        assert!(validate_config_generic(&c).is_ok());
        c.timeout_secs = 1;
        assert!(validate_config_generic(&c).is_err());
        c.timeout_secs = 60;
        c.temperature = Some(f32::NAN);
        assert!(validate_config_generic(&c).is_err());
        c.temperature = Some(2.5);
        assert!(validate_config_generic(&c).is_err());
        c.temperature = Some(0.7);
        c.max_output_tokens = Some(0);
        assert!(validate_config_generic(&c).is_err());
        c.max_output_tokens = Some(1000);
        c.default_model = Some("bad model".into());
        assert!(validate_config_generic(&c).is_err());
        c.default_model = Some("good-model".into());
        assert!(validate_config_generic(&c).is_ok());
        assert!(serde_json::from_str::<ProviderConfig>(r#"{"provider":"openai","api_key":"sk-leak-12345678"}"#).is_err(), "a key cannot ride along in a config");
    }

    #[test]
    fn every_error_kind_has_a_code_and_a_recovery_and_only_transient_ones_retry() {
        use AiErrorKind::*;
        for k in [Authentication, Permission, Quota, RateLimit, Network, Timeout, ModelNotFound, InvalidRequest, ContentFiltered, Server, Overloaded, Cancelled, Unsupported, Configuration, CredentialStore, Protocol, TooLarge] {
            assert!(k.code().starts_with("ai_") && !k.recovery().is_empty());
        }
        for k in [Authentication, Permission, Quota, ModelNotFound, InvalidRequest, ContentFiltered, Cancelled, Unsupported, Configuration, CredentialStore, Protocol, TooLarge] {
            assert!(!k.retryable(), "{k:?}");
        }
        for k in [RateLimit, Network, Timeout, Server, Overloaded] {
            assert!(k.retryable(), "{k:?}");
        }
    }
}
