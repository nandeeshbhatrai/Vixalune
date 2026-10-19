//! Anthropic, through the Messages API (`POST /v1/messages` with `stream: true`) and `GET /v1/models`.
//! Documented at https://platform.claude.com/docs (Messages, Streaming). Requests carry `x-api-key` and
//! `anthropic-version: 2023-06-01`.

use super::common::{error_message, parse_json, str_at, u64_at, validate_common, Emitter};
use super::http::{self, read_json, send, stream_sse, Step, StreamEnd};
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::ports::{AiProvider, StreamSink};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

pub const ID: &str = "anthropic";
const DEFAULT_BASE: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const MAX_MODELS_BYTES: usize = 4 * 1024 * 1024;
const MAX_MODEL_PAGES: usize = 5;

pub struct Anthropic {
    base: String,
    client: reqwest::Client,
}

impl Anthropic {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_BASE).expect("the default address is valid")
    }

    pub fn with_base_url(base: &str) -> AiResult<Self> {
        http::check_base_url(base)?;
        Ok(Self { base: base.trim_end_matches('/').to_string(), client: http::build_client() })
    }

    fn rb(&self, method: reqwest::Method, path: &str, key: &Secret) -> reqwest::RequestBuilder {
        self.client.request(method, format!("{}{path}", self.base)).header("x-api-key", key.expose()).header("anthropic-version", API_VERSION)
    }
}

impl Default for Anthropic {
    fn default() -> Self {
        Self::new()
    }
}

pub fn build_body(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req.messages.iter().map(|m| json!({"role": if m.role == Role::Assistant { "assistant" } else { "user" }, "content": m.content})).collect();
    // `max_tokens` is required by this API.
    let mut body = json!({"model": req.model, "max_tokens": req.params.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS), "messages": messages, "stream": true});
    let o = body.as_object_mut().unwrap();
    if let Some(s) = &req.system {
        o.insert("system".into(), json!(s));
    }
    if let Some(t) = req.params.temperature {
        o.insert("temperature".into(), json!(t));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.parameters})).collect();
        o.insert("tools".into(), Value::Array(tools));
    }
    body
}

fn kind_from_type(ty: &str, message: &str) -> AiErrorKind {
    let m = message.to_ascii_lowercase();
    match ty {
        "authentication_error" => AiErrorKind::Authentication,
        "permission_error" => AiErrorKind::Permission,
        "not_found_error" => AiErrorKind::ModelNotFound,
        "rate_limit_error" => AiErrorKind::RateLimit,
        "overloaded_error" => AiErrorKind::Overloaded,
        "api_error" => AiErrorKind::Server,
        "request_too_large" => AiErrorKind::TooLarge,
        "billing_error" => AiErrorKind::Quota,
        "invalid_request_error" => {
            if m.contains("credit balance") || m.contains("billing") {
                AiErrorKind::Quota
            } else {
                AiErrorKind::InvalidRequest
            }
        }
        _ => AiErrorKind::Server,
    }
}

fn parse_error(status: u16, body: &str) -> (AiErrorKind, String) {
    let msg = error_message(body).unwrap_or_default();
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let ty = str_at(&v, &["error", "type"]).unwrap_or("");
    let kind = if ty.is_empty() { http::kind_from_status(status) } else { kind_from_type(ty, &msg) };
    // The status decides the transient classes even if the body is odd.
    let kind = match status {
        429 => if kind == AiErrorKind::Quota { kind } else { AiErrorKind::RateLimit },
        529 => AiErrorKind::Overloaded,
        500..=599 => if kind == AiErrorKind::Overloaded { kind } else { AiErrorKind::Server },
        401 => AiErrorKind::Authentication,
        403 => AiErrorKind::Permission,
        _ => kind,
    };
    (kind, msg)
}

fn finish_reason(s: &str) -> FinishReason {
    match s {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolCalls,
        "refusal" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

#[derive(Default)]
struct Parser {
    tool_index: HashMap<u64, usize>,
    finished: bool,
}

impl Parser {
    fn handle(&mut self, data: &str, em: &mut Emitter<'_>) -> AiResult<Step> {
        let v = parse_json(data)?;
        match str_at(&v, &["type"]).unwrap_or("") {
            "message_start" => {
                em.emit(StreamEvent::Started { model: str_at(&v, &["message", "model"]).map(String::from) });
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    em.emit(StreamEvent::Usage { usage: Usage { input_tokens: u64_at(u, &["input_tokens"]), output_tokens: None, total_tokens: None, cost_usd: None } });
                }
            }
            "content_block_start" => {
                if str_at(&v, &["content_block", "type"]) == Some("tool_use") {
                    let block = u64_at(&v, &["index"]).unwrap_or(0);
                    let idx = self.tool_index.len();
                    self.tool_index.insert(block, idx);
                    em.emit(StreamEvent::ToolCallStart { index: idx, id: str_at(&v, &["content_block", "id"]).unwrap_or("").into(), name: str_at(&v, &["content_block", "name"]).unwrap_or("").into() });
                }
            }
            "content_block_delta" => match str_at(&v, &["delta", "type"]) {
                Some("text_delta") => {
                    if let Some(t) = str_at(&v, &["delta", "text"]).filter(|t| !t.is_empty()) {
                        if !em.emit(StreamEvent::TextDelta { text: t.to_string() }) {
                            self.finished = true;
                            return Ok(Step::Done);
                        }
                    }
                }
                Some("input_json_delta") => {
                    let block = u64_at(&v, &["index"]).unwrap_or(0);
                    if let (Some(idx), Some(f)) = (self.tool_index.get(&block), str_at(&v, &["delta", "partial_json"])) {
                        em.emit(StreamEvent::ToolCallArguments { index: *idx, fragment: f.to_string() });
                    }
                }
                _ => {} // thinking and signature deltas are not shown
            },
            "message_delta" => {
                if let Some(u) = v.get("usage") {
                    em.emit(StreamEvent::Usage { usage: Usage { input_tokens: u64_at(u, &["input_tokens"]), output_tokens: u64_at(u, &["output_tokens"]), total_tokens: None, cost_usd: None } });
                }
                if let Some(r) = str_at(&v, &["delta", "stop_reason"]) {
                    self.finished = true;
                    em.emit(StreamEvent::Finished { reason: finish_reason(r) });
                }
            }
            "message_stop" => return Ok(Step::Done),
            "error" => {
                let msg = str_at(&v, &["error", "message"]).unwrap_or("the provider reported an error");
                return Err(AiError::new(kind_from_type(str_at(&v, &["error", "type"]).unwrap_or(""), msg), msg));
            }
            _ => {} // ping and unknown future events
        }
        Ok(Step::Continue)
    }
}

fn parse_models(v: &Value) -> Vec<ModelInfo> {
    v.get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = str_at(m, &["id"])?;
            valid_model_id(id).then(|| ModelInfo { id: id.to_string(), display_name: str_at(m, &["display_name"]).map(String::from), context_window: None, max_output_tokens: None, supports_tools: None, pricing: None, source: ModelSource::Listed })
        })
        .collect()
}

#[async_trait]
impl AiProvider for Anthropic {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: ID.into(),
            display_name: "Anthropic".into(),
            capabilities: ProviderCapabilities { streaming: true, tool_calling: true, structured_output: false, model_listing: true, usage_reporting: true, cost_reporting: false, cancellation: true, context_limits: false },
            key_hint: "Keys start with sk-ant-".into(),
            key_page: "https://console.anthropic.com/settings/keys".into(),
            docs_page: "https://docs.anthropic.com".into(),
            billing_note: "Requests are billed to your Anthropic API account by tokens. A Claude subscription does not include API use. Anthropic does not report a price per response, so Vixalune shows token counts only.".into(),
            verification_note: "Implemented against the documented Messages streaming events (checked against the documentation on 2026-10-20) and /v1/models. Not yet verified with a real key.".into(),
            extra_settings: vec![],
        }
    }

    fn validate_config(&self, config: &ProviderConfig) -> AiResult<()> {
        validate_config_generic(config)?;
        if config.temperature.is_some_and(|t| t > 1.0) {
            return Err(AiError::new(AiErrorKind::Configuration, "for Anthropic the temperature must be between 0 and 1"));
        }
        if !config.extra.is_empty() {
            return Err(AiError::new(AiErrorKind::Configuration, "Anthropic has no extra settings"));
        }
        Ok(())
    }

    fn validate_request(&self, req: &ChatRequest) -> AiResult<()> {
        validate_common(req)?;
        if req.params.temperature.is_some_and(|t| t > 1.0) {
            return Err(AiError::new(AiErrorKind::InvalidRequest, "for Anthropic the temperature must be between 0 and 1"));
        }
        Ok(())
    }

    async fn check_auth(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<()> {
        let resp = send(self.rb(reqwest::Method::GET, "/v1/models?limit=1", key), key, cancel, timeout, parse_error).await?;
        read_json(resp, key, cancel, timeout, MAX_MODELS_BYTES).await.map(|_| ())
    }

    async fn list_models(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<Vec<ModelInfo>> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let path = match &after {
                Some(a) if valid_model_id(a) => format!("/v1/models?limit=1000&after_id={a}"),
                _ => "/v1/models?limit=1000".to_string(),
            };
            let resp = send(self.rb(reqwest::Method::GET, &path, key), key, cancel, timeout, parse_error).await?;
            let v = read_json(resp, key, cancel, timeout, MAX_MODELS_BYTES).await?;
            out.extend(parse_models(&v));
            let more = v.get("has_more").and_then(|h| h.as_bool()).unwrap_or(false);
            after = str_at(&v, &["last_id"]).map(String::from);
            if !more || after.is_none() {
                break;
            }
        }
        if out.is_empty() {
            return Err(AiError::new(AiErrorKind::Protocol, "the provider returned no models"));
        }
        Ok(out)
    }

    async fn chat_stream(&self, key: &Secret, req: &ChatRequest, _config: &ProviderConfig, sink: &dyn StreamSink, cancel: &CancellationToken) -> AiResult<ChatOutcome> {
        self.validate_request(req)?;
        let rb = self.rb(reqwest::Method::POST, "/v1/messages", key).json(&build_body(req));
        let limit = Duration::from_secs(u64::from(req.idle_timeout_secs));
        let resp = send(rb, key, cancel, limit, parse_error).await?;
        let mut em = Emitter::new(sink);
        let mut parser = Parser::default();
        let end = stream_sse(resp, key, cancel, limit, |ev| parser.handle(&ev.data, &mut em)).await.map_err(|e| e.scrubbed(key))?;
        if end == StreamEnd::Closed && !parser.finished {
            return Err(AiError::new(AiErrorKind::Network, "the connection closed before the response was complete"));
        }
        Ok(em.into_outcome())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ai::common::VecSink;
    use crate::domain::ai::{ChatMessage, GenerationParams, ToolDescription};

    fn req() -> ChatRequest {
        ChatRequest { model: "claude-x".into(), system: Some("be brief".into()), messages: vec![ChatMessage::user("hi")], tools: vec![], params: GenerationParams::default(), idle_timeout_secs: 30 }
    }

    #[test]
    fn the_body_always_has_max_tokens_and_puts_the_system_prompt_in_its_own_field() {
        let b = build_body(&req());
        assert_eq!(b["max_tokens"], DEFAULT_MAX_OUTPUT_TOKENS);
        assert_eq!(b["system"], "be brief");
        assert_eq!(b["messages"], json!([{"role": "user", "content": "hi"}]));
        assert_eq!(b["stream"], true);
        let mut r = req();
        r.params = GenerationParams { temperature: Some(0.3), max_output_tokens: Some(256) };
        r.tools = vec![ToolDescription { name: "t".into(), description: "d".into(), parameters: json!({"type": "object"}) }];
        let b = build_body(&r);
        assert_eq!((b["max_tokens"].as_u64(), b["temperature"].as_f64().map(|t| (t * 10.0).round() as i64)), (Some(256), Some(3)));
        assert_eq!(b["tools"][0]["input_schema"], json!({"type": "object"}));
    }

    #[test]
    fn the_documented_event_sequence_is_assembled_including_ping_and_usage() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        for d in [
            r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-x","usage":{"input_tokens":25,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hidden"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":15}}"#,
        ] {
            assert_eq!(p.handle(d, &mut em).unwrap(), Step::Continue);
        }
        assert_eq!(p.handle(r#"{"type":"message_stop"}"#, &mut em).unwrap(), Step::Done);
        let o = em.into_outcome();
        assert_eq!(o.text, "Hello!");
        assert_eq!(o.finish, Some(FinishReason::Stop));
        assert_eq!((o.usage.input_tokens, o.usage.output_tokens, o.usage.total_tokens), (Some(25), Some(15), Some(40)));
        assert_eq!(o.model.as_deref(), Some("claude-x"));
        assert!(!o.text.contains("hidden"));
    }

    #[test]
    fn tool_use_blocks_and_stop_reasons() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        for d in [
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu1","name":"lookup"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"1}"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}"#,
        ] {
            p.handle(d, &mut em).unwrap();
        }
        let o = em.into_outcome();
        assert_eq!((o.tool_calls[0].name.as_str(), o.tool_calls[0].arguments.as_str()), ("lookup", "{\"q\":1}"));
        assert_eq!(o.finish, Some(FinishReason::ToolCalls));
        for (s, f) in [("max_tokens", FinishReason::Length), ("refusal", FinishReason::ContentFilter), ("stop_sequence", FinishReason::Stop)] {
            assert_eq!(finish_reason(s), f);
        }
    }

    #[test]
    fn stream_and_http_errors_are_classified() {
        let sink = VecSink::default();
        let e = Parser::default().handle(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#, &mut Emitter::new(&sink)).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Overloaded);
        assert!(e.retryable());
        assert_eq!(parse_error(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#).0, AiErrorKind::Authentication);
        assert_eq!(parse_error(403, r#"{"error":{"type":"permission_error","message":"x"}}"#).0, AiErrorKind::Permission);
        assert_eq!(parse_error(404, r#"{"error":{"type":"not_found_error","message":"model: x"}}"#).0, AiErrorKind::ModelNotFound);
        assert_eq!(parse_error(429, r#"{"error":{"type":"rate_limit_error","message":"x"}}"#).0, AiErrorKind::RateLimit);
        assert_eq!(parse_error(400, r#"{"error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the API"}}"#).0, AiErrorKind::Quota);
        assert_eq!(parse_error(400, r#"{"error":{"type":"invalid_request_error","message":"max_tokens: too large"}}"#).0, AiErrorKind::InvalidRequest);
        assert_eq!(parse_error(529, "").0, AiErrorKind::Overloaded);
        assert_eq!(parse_error(500, r#"{"error":{"type":"api_error","message":"x"}}"#).0, AiErrorKind::Server);
    }

    #[test]
    fn temperature_ranges_follow_the_provider() {
        let p = Anthropic::new();
        let mut c = ProviderConfig::new(ID);
        c.temperature = Some(1.5);
        assert!(p.validate_config(&c).is_err());
        c.temperature = Some(1.0);
        assert!(p.validate_config(&c).is_ok());
        let mut r = req();
        r.params.temperature = Some(1.2);
        assert!(p.validate_request(&r).is_err());
    }

    #[test]
    fn models_are_parsed() {
        let m = parse_models(&json!({"data":[{"id":"claude-a","display_name":"Claude A"},{"id":"bad id"}],"has_more":false}));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].display_name.as_deref(), Some("Claude A"));
    }
}
