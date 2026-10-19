//! OpenAI, through the Responses API (`POST /v1/responses` with `stream: true`) and `GET /v1/models`.
//! Documented at https://platform.openai.com/docs/api-reference/responses . Responses are requested with `store: false`
//! so OpenAI is asked not to keep them for later retrieval.

use super::common::{error_message, parse_json, str_at, u64_at, validate_common, Emitter};
use super::http::{self, read_json, send, stream_sse, Step, StreamEnd};
use super::openai_compat::kind_from_code;
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::ports::{AiProvider, StreamSink};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

pub const ID: &str = "openai";
const DEFAULT_BASE: &str = "https://api.openai.com/v1";
const MAX_MODELS_BYTES: usize = 8 * 1024 * 1024;

pub struct OpenAi {
    base: String,
    client: reqwest::Client,
}

impl OpenAi {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_BASE).expect("the default address is valid")
    }

    pub fn with_base_url(base: &str) -> AiResult<Self> {
        http::check_base_url(base)?;
        Ok(Self { base: base.trim_end_matches('/').to_string(), client: http::build_client() })
    }
}

impl Default for OpenAi {
    fn default() -> Self {
        Self::new()
    }
}

pub fn build_body(req: &ChatRequest) -> Value {
    let input: Vec<Value> = req
        .messages
        .iter()
        .map(|m| json!({"role": if m.role == Role::Assistant { "assistant" } else { "user" }, "content": m.content}))
        .collect();
    let mut body = json!({"model": req.model, "input": input, "stream": true, "store": false});
    let o = body.as_object_mut().unwrap();
    if let Some(s) = &req.system {
        o.insert("instructions".into(), json!(s));
    }
    if let Some(t) = req.params.temperature {
        o.insert("temperature".into(), json!(t));
    }
    if let Some(n) = req.params.max_output_tokens {
        o.insert("max_output_tokens".into(), json!(n));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req.tools.iter().map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.parameters})).collect();
        o.insert("tools".into(), Value::Array(tools));
    }
    body
}

fn parse_error(status: u16, body: &str) -> (AiErrorKind, String) {
    let msg = error_message(body).unwrap_or_default();
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v.get("error").and_then(|e| e.get("code")).cloned().unwrap_or(Value::Null);
    let by_code = kind_from_code(&code, &msg);
    let kind = match status {
        401 => AiErrorKind::Authentication,
        403 => AiErrorKind::Permission,
        404 => AiErrorKind::ModelNotFound,
        // A 429 is a rate limit unless the account is out of credit.
        429 => {
            if by_code == AiErrorKind::Quota {
                AiErrorKind::Quota
            } else {
                AiErrorKind::RateLimit
            }
        }
        400 | 422 => {
            if matches!(by_code, AiErrorKind::ModelNotFound | AiErrorKind::TooLarge | AiErrorKind::Authentication | AiErrorKind::Quota) {
                by_code
            } else {
                AiErrorKind::InvalidRequest
            }
        }
        s => http::kind_from_status(s),
    };
    (kind, msg)
}

fn response_failure(resp: &Value) -> AiError {
    let msg = str_at(resp, &["error", "message"]).unwrap_or("the response failed");
    let code = resp.get("error").and_then(|e| e.get("code")).cloned().unwrap_or(Value::Null);
    let mut kind = kind_from_code(&code, msg);
    if let Some(c) = code.as_str() {
        if c == "server_error" || c == "rate_limit_exceeded" {
            kind = if c == "server_error" { AiErrorKind::Server } else { AiErrorKind::RateLimit };
        }
    }
    AiError::new(kind, msg)
}

#[derive(Default)]
struct Parser {
    tool_index: HashMap<u64, usize>,
    started: bool,
    finished: bool,
}

impl Parser {
    fn handle(&mut self, data: &str, em: &mut Emitter<'_>) -> AiResult<Step> {
        let v = parse_json(data)?;
        let ty = str_at(&v, &["type"]).unwrap_or("");
        match ty {
            "response.created" => {
                if !self.started {
                    self.started = true;
                    em.emit(StreamEvent::Started { model: str_at(&v, &["response", "model"]).map(String::from) });
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(d) = str_at(&v, &["delta"]).filter(|d| !d.is_empty()) {
                    if !em.emit(StreamEvent::TextDelta { text: d.to_string() }) {
                        self.finished = true;
                        return Ok(Step::Done);
                    }
                }
            }
            "response.output_item.added" => {
                if str_at(&v, &["item", "type"]) == Some("function_call") {
                    let out_idx = u64_at(&v, &["output_index"]).unwrap_or(0);
                    let idx = self.tool_index.len();
                    self.tool_index.insert(out_idx, idx);
                    em.emit(StreamEvent::ToolCallStart { index: idx, id: str_at(&v, &["item", "call_id"]).or(str_at(&v, &["item", "id"])).unwrap_or("").to_string(), name: str_at(&v, &["item", "name"]).unwrap_or("").to_string() });
                }
            }
            "response.function_call_arguments.delta" => {
                let out_idx = u64_at(&v, &["output_index"]).unwrap_or(0);
                if let (Some(idx), Some(d)) = (self.tool_index.get(&out_idx), str_at(&v, &["delta"])) {
                    em.emit(StreamEvent::ToolCallArguments { index: *idx, fragment: d.to_string() });
                }
            }
            "response.completed" | "response.incomplete" => {
                let r = v.get("response").unwrap_or(&Value::Null);
                if let Some(u) = r.get("usage").filter(|u| u.is_object()) {
                    em.emit(StreamEvent::Usage { usage: Usage { input_tokens: u64_at(u, &["input_tokens"]), output_tokens: u64_at(u, &["output_tokens"]), total_tokens: u64_at(u, &["total_tokens"]), cost_usd: None } });
                }
                let reason = if ty == "response.incomplete" {
                    match str_at(r, &["incomplete_details", "reason"]) {
                        Some("max_output_tokens") => FinishReason::Length,
                        Some("content_filter") => FinishReason::ContentFilter,
                        Some(o) => FinishReason::Other(o.to_string()),
                        None => FinishReason::Other("incomplete".into()),
                    }
                } else if !self.tool_index.is_empty() {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                };
                self.finished = true;
                em.emit(StreamEvent::Finished { reason });
                return Ok(Step::Done);
            }
            "response.failed" => return Err(response_failure(v.get("response").unwrap_or(&Value::Null))),
            "error" => {
                let msg = str_at(&v, &["message"]).or(str_at(&v, &["error", "message"])).unwrap_or("the provider reported an error");
                let code = v.get("code").or_else(|| v.get("error").and_then(|e| e.get("code"))).cloned().unwrap_or(Value::Null);
                return Err(AiError::new(kind_from_code(&code, msg), msg));
            }
            _ => {} // many other progress events carry nothing Vixalune shows
        }
        Ok(Step::Continue)
    }
}

fn is_chat_model(id: &str) -> bool {
    const SKIP: &[&str] = &["embedding", "whisper", "tts", "dall-e", "moderation", "transcribe", "gpt-image", "realtime", "audio", "sora", "babbage", "davinci", "computer-use"];
    !SKIP.iter().any(|s| id.contains(s))
}

fn parse_models(v: &Value) -> Vec<ModelInfo> {
    v.get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| str_at(m, &["id"]))
        .filter(|id| valid_model_id(id) && is_chat_model(id))
        .map(|id| ModelInfo { id: id.to_string(), display_name: None, context_window: None, max_output_tokens: None, supports_tools: None, pricing: None, source: ModelSource::Listed })
        .collect()
}

#[async_trait]
impl AiProvider for OpenAi {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: ID.into(),
            display_name: "OpenAI".into(),
            capabilities: ProviderCapabilities { streaming: true, tool_calling: true, structured_output: true, model_listing: true, usage_reporting: true, cost_reporting: false, cancellation: true, context_limits: false },
            key_hint: "Keys start with sk-".into(),
            key_page: "https://platform.openai.com/api-keys".into(),
            docs_page: "https://platform.openai.com/docs/api-reference".into(),
            billing_note: "Requests are billed to your OpenAI API account by tokens. A ChatGPT subscription does not include API use. OpenAI does not report a price per response, so Vixalune shows token counts only.".into(),
            verification_note: "Implemented against the documented Responses API streaming events and /v1/models. Not yet verified with a real key.".into(),
            extra_settings: vec![],
        }
    }

    fn validate_config(&self, config: &ProviderConfig) -> AiResult<()> {
        validate_config_generic(config)?;
        if !config.extra.is_empty() {
            return Err(AiError::new(AiErrorKind::Configuration, "OpenAI has no extra settings"));
        }
        Ok(())
    }

    fn validate_request(&self, req: &ChatRequest) -> AiResult<()> {
        validate_common(req)?;
        if req.params.max_output_tokens.is_some_and(|n| n < 16) {
            return Err(AiError::new(AiErrorKind::InvalidRequest, "OpenAI needs a response length of at least 16 tokens"));
        }
        Ok(())
    }

    async fn check_auth(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<()> {
        let rb = self.client.get(format!("{}/models", self.base)).bearer_auth(key.expose());
        let resp = send(rb, key, cancel, timeout, parse_error).await?;
        read_json(resp, key, cancel, timeout, MAX_MODELS_BYTES).await.map(|_| ())
    }

    async fn list_models(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<Vec<ModelInfo>> {
        let rb = self.client.get(format!("{}/models", self.base)).bearer_auth(key.expose());
        let resp = send(rb, key, cancel, timeout, parse_error).await?;
        let v = read_json(resp, key, cancel, timeout, MAX_MODELS_BYTES).await?;
        let mut models = parse_models(&v);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        if models.is_empty() {
            return Err(AiError::new(AiErrorKind::Protocol, "the provider returned no usable models"));
        }
        Ok(models)
    }

    async fn chat_stream(&self, key: &Secret, req: &ChatRequest, _config: &ProviderConfig, sink: &dyn StreamSink, cancel: &CancellationToken) -> AiResult<ChatOutcome> {
        self.validate_request(req)?;
        let rb = self.client.post(format!("{}/responses", self.base)).bearer_auth(key.expose()).json(&build_body(req));
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
    use crate::domain::ai::{ChatMessage, GenerationParams};

    fn req() -> ChatRequest {
        ChatRequest { model: "some-model".into(), system: Some("be brief".into()), messages: vec![ChatMessage::user("hi"), ChatMessage::assistant("yo"), ChatMessage::user("more")], tools: vec![], params: GenerationParams { temperature: None, max_output_tokens: Some(64) }, idle_timeout_secs: 30 }
    }

    #[test]
    fn the_request_uses_the_responses_shape_and_asks_not_to_store() {
        let b = build_body(&req());
        assert_eq!(b["model"], "some-model");
        assert_eq!(b["stream"], true);
        assert_eq!(b["store"], false);
        assert_eq!(b["instructions"], "be brief");
        assert_eq!(b["input"][1], json!({"role": "assistant", "content": "yo"}));
        assert_eq!(b["max_output_tokens"], 64);
        assert!(b.get("temperature").is_none(), "temperature is only sent when the user chose one (some models reject it)");
        assert!(!b.to_string().contains("api_key"));
    }

    #[test]
    fn a_response_stream_with_usage_is_assembled() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        for d in [
            r#"{"type":"response.created","response":{"model":"some-model"}}"#,
            r#"{"type":"response.in_progress"}"#,
            r#"{"type":"response.output_text.delta","delta":"Hel"}"#,
            r#"{"type":"response.output_text.delta","delta":"lo"}"#,
            r#"{"type":"response.output_text.done","text":"Hello"}"#,
        ] {
            assert_eq!(p.handle(d, &mut em).unwrap(), Step::Continue);
        }
        let done = r#"{"type":"response.completed","response":{"usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}"#;
        assert_eq!(p.handle(done, &mut em).unwrap(), Step::Done);
        let o = em.into_outcome();
        assert_eq!(o.text, "Hello");
        assert_eq!(o.finish, Some(FinishReason::Stop));
        assert_eq!((o.usage.input_tokens, o.usage.output_tokens, o.usage.total_tokens, o.usage.cost_usd), (Some(7), Some(3), Some(10), None));
    }

    #[test]
    fn incomplete_failed_and_error_events_are_classified() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        assert_eq!(p.handle(r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#, &mut em).unwrap(), Step::Done);
        assert_eq!(em.outcome.finish, Some(FinishReason::Length));
        let e = Parser::default().handle(r#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded","message":"slow"}}}"#, &mut Emitter::new(&sink)).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::RateLimit);
        let e = Parser::default().handle(r#"{"type":"error","code":"insufficient_quota","message":"no credit"}"#, &mut Emitter::new(&sink)).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Quota);
        assert_eq!(Parser::default().handle("garbage", &mut Emitter::new(&sink)).unwrap_err().kind, AiErrorKind::Protocol);
    }

    #[test]
    fn function_calls_are_collected() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        for d in [
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"c9","name":"lookup"}}"#,
            r#"{"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"a\":"}"#,
            r#"{"type":"response.function_call_arguments.delta","output_index":1,"delta":"2}"}"#,
            r#"{"type":"response.completed","response":{}}"#,
        ] {
            p.handle(d, &mut em).unwrap();
        }
        let o = em.into_outcome();
        assert_eq!((o.tool_calls[0].id.as_str(), o.tool_calls[0].arguments.as_str()), ("c9", "{\"a\":2}"));
        assert_eq!(o.finish, Some(FinishReason::ToolCalls));
    }

    #[test]
    fn errors_are_classified_by_status_and_code() {
        assert_eq!(parse_error(401, r#"{"error":{"code":"invalid_api_key","message":"Incorrect API key provided: sk-xxx"}}"#).0, AiErrorKind::Authentication);
        assert_eq!(parse_error(429, r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota"}}"#).0, AiErrorKind::Quota);
        assert_eq!(parse_error(429, r#"{"error":{"code":"rate_limit_exceeded","message":"x"}}"#).0, AiErrorKind::RateLimit);
        assert_eq!(parse_error(404, r#"{"error":{"code":"model_not_found","message":"x"}}"#).0, AiErrorKind::ModelNotFound);
        assert_eq!(parse_error(400, r#"{"error":{"code":"context_length_exceeded","message":"x"}}"#).0, AiErrorKind::TooLarge);
        assert_eq!(parse_error(500, "").0, AiErrorKind::Server);
    }

    #[test]
    fn the_model_list_keeps_chat_capable_ids_and_drops_other_families() {
        let v = json!({"data":[{"id":"gpt-a"},{"id":"text-embedding-x"},{"id":"whisper-1"},{"id":"o-reasoner"},{"id":"dall-e-3"},{"id":"tts-1"},{"id":"bad id"}]});
        let ids: Vec<String> = parse_models(&v).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["gpt-a", "o-reasoner"]);
    }

    #[test]
    fn request_validation_catches_unsupported_and_malformed_input() {
        let p = OpenAi::new();
        assert!(p.validate_request(&req()).is_ok());
        let mut r = req();
        r.params.max_output_tokens = Some(4);
        assert!(p.validate_request(&r).is_err());
        let mut r = req();
        r.messages.push(ChatMessage { role: Role::Tool, content: "x".into(), tool_calls: vec![], tool_call_id: Some("1".into()) });
        assert_eq!(p.validate_request(&r).unwrap_err().kind, AiErrorKind::Unsupported);
        let mut r = req();
        r.model = "bad model!".into();
        assert_eq!(p.validate_request(&r).unwrap_err().kind, AiErrorKind::ModelNotFound);
        let mut r = req();
        r.messages.pop();
        assert!(p.validate_request(&r).is_err(), "must end with a user message");
        assert!(p.validate_config(&{
            let mut c = ProviderConfig::new(ID);
            c.extra.insert("x".into(), "y".into());
            c
        })
        .is_err());
    }
}
