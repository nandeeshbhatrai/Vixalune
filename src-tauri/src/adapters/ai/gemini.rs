//! Google Gemini, through the Gemini API (`models.list`, `models/{model}:streamGenerateContent?alt=sse`).
//! Documented at https://ai.google.dev/api . The key travels in the `x-goog-api-key` header, never in the URL, so it cannot
//! end up in logs, proxies or error messages that print the address.

use super::common::{error_message, parse_json, str_at, u64_at, validate_common, Emitter};
use super::http::{self, read_json, send, stream_sse, Step, StreamEnd};
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::ports::{AiProvider, StreamSink};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub const ID: &str = "gemini";
const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const MAX_MODELS_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODEL_PAGES: usize = 5;

pub struct Gemini {
    base: String,
    client: reqwest::Client,
}

impl Gemini {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_BASE).expect("the default address is valid")
    }

    pub fn with_base_url(base: &str) -> AiResult<Self> {
        http::check_base_url(base)?;
        Ok(Self { base: base.trim_end_matches('/').to_string(), client: http::build_client() })
    }

    fn get(&self, path: &str, key: &Secret) -> reqwest::RequestBuilder {
        self.client.get(format!("{}{path}", self.base)).header("x-goog-api-key", key.expose())
    }
}

impl Default for Gemini {
    fn default() -> Self {
        Self::new()
    }
}

fn model_path_ok(id: &str) -> bool {
    valid_model_id(id) && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn build_body(req: &ChatRequest) -> Value {
    let contents: Vec<Value> = req.messages.iter().map(|m| json!({"role": if m.role == Role::Assistant { "model" } else { "user" }, "parts": [{"text": m.content}]})).collect();
    let mut body = json!({"contents": contents});
    let o = body.as_object_mut().unwrap();
    if let Some(s) = &req.system {
        o.insert("systemInstruction".into(), json!({"parts": [{"text": s}]}));
    }
    let mut gen = serde_json::Map::new();
    if let Some(t) = req.params.temperature {
        gen.insert("temperature".into(), json!(t));
    }
    if let Some(n) = req.params.max_output_tokens {
        gen.insert("maxOutputTokens".into(), json!(n));
    }
    if !gen.is_empty() {
        o.insert("generationConfig".into(), Value::Object(gen));
    }
    if !req.tools.is_empty() {
        let decls: Vec<Value> = req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "parameters": t.parameters})).collect();
        o.insert("tools".into(), json!([{"functionDeclarations": decls}]));
    }
    body
}

fn parse_error(status: u16, body: &str) -> (AiErrorKind, String) {
    let msg = error_message(body).unwrap_or_default();
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let st = str_at(&v, &["error", "status"]).unwrap_or("");
    let reason_invalid_key = v.get("error").and_then(|e| e.get("details")).and_then(|d| d.as_array()).is_some_and(|a| a.iter().any(|x| str_at(x, &["reason"]) == Some("API_KEY_INVALID")));
    let m = msg.to_ascii_lowercase();
    let kind = if reason_invalid_key || m.contains("api key not valid") || m.contains("api key expired") || st == "UNAUTHENTICATED" {
        AiErrorKind::Authentication
    } else {
        match st {
            "PERMISSION_DENIED" | "FAILED_PRECONDITION" => AiErrorKind::Permission,
            "NOT_FOUND" => AiErrorKind::ModelNotFound,
            "RESOURCE_EXHAUSTED" => {
                if m.contains("billing") || m.contains("per day") || m.contains("current quota") {
                    AiErrorKind::Quota
                } else {
                    AiErrorKind::RateLimit
                }
            }
            "UNAVAILABLE" => AiErrorKind::Overloaded,
            "DEADLINE_EXCEEDED" => AiErrorKind::Timeout,
            "INTERNAL" => AiErrorKind::Server,
            "INVALID_ARGUMENT" => AiErrorKind::InvalidRequest,
            _ => http::kind_from_status(status),
        }
    };
    (kind, msg)
}

fn finish_reason(s: &str) -> FinishReason {
    match s {
        "STOP" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "RECITATION" | "IMAGE_SAFETY" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

#[derive(Default)]
struct Parser {
    started: bool,
    finished: bool,
    tool_calls: usize,
}

impl Parser {
    fn handle(&mut self, data: &str, em: &mut Emitter<'_>) -> AiResult<Step> {
        let v = parse_json(data)?;
        if let Some(err) = v.get("error") {
            let body = json!({"error": err}).to_string();
            let (kind, msg) = parse_error(err.get("code").and_then(|c| c.as_u64()).unwrap_or(500) as u16, &body);
            return Err(AiError::new(kind, msg));
        }
        if !self.started {
            self.started = true;
            em.emit(StreamEvent::Started { model: str_at(&v, &["modelVersion"]).map(String::from) });
        }
        if let Some(block) = str_at(&v, &["promptFeedback", "blockReason"]) {
            return Err(AiError::new(AiErrorKind::ContentFiltered, format!("the provider blocked the request ({block})")));
        }
        for cand in v.get("candidates").and_then(|c| c.as_array()).into_iter().flatten().take(1) {
            for part in cand.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()).into_iter().flatten() {
                if part.get("thought").and_then(|t| t.as_bool()) == Some(true) {
                    continue; // reasoning summaries are not shown
                }
                if let Some(t) = str_at(part, &["text"]).filter(|t| !t.is_empty()) {
                    if !em.emit(StreamEvent::TextDelta { text: t.to_string() }) {
                        self.finished = true;
                        return Ok(Step::Done);
                    }
                }
                if let Some(fc) = part.get("functionCall") {
                    let idx = self.tool_calls;
                    self.tool_calls += 1;
                    em.emit(StreamEvent::ToolCallStart { index: idx, id: format!("call_{idx}"), name: str_at(fc, &["name"]).unwrap_or("").into() });
                    em.emit(StreamEvent::ToolCallArguments { index: idx, fragment: fc.get("args").map(|a| a.to_string()).unwrap_or_else(|| "{}".into()) });
                }
            }
            if let Some(r) = str_at(cand, &["finishReason"]) {
                self.finished = true;
                let reason = if self.tool_calls > 0 && r == "STOP" { FinishReason::ToolCalls } else { finish_reason(r) };
                em.emit(StreamEvent::Finished { reason });
            }
        }
        if let Some(u) = v.get("usageMetadata") {
            let out = u64_at(u, &["candidatesTokenCount"]).unwrap_or(0) + u64_at(u, &["thoughtsTokenCount"]).unwrap_or(0);
            em.emit(StreamEvent::Usage { usage: Usage { input_tokens: u64_at(u, &["promptTokenCount"]), output_tokens: if u.get("candidatesTokenCount").is_some() || u.get("thoughtsTokenCount").is_some() { Some(out) } else { None }, total_tokens: u64_at(u, &["totalTokenCount"]), cost_usd: None } });
        }
        Ok(Step::Continue)
    }
}

fn parse_models(v: &Value) -> Vec<ModelInfo> {
    v.get("models")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter(|m| m.get("supportedGenerationMethods").and_then(|s| s.as_array()).is_some_and(|a| a.iter().any(|x| x.as_str() == Some("generateContent"))))
        .filter_map(|m| {
            let id = str_at(m, &["name"])?.strip_prefix("models/")?;
            model_path_ok(id).then(|| ModelInfo {
                id: id.to_string(),
                display_name: str_at(m, &["displayName"]).map(String::from),
                context_window: u64_at(m, &["inputTokenLimit"]),
                max_output_tokens: u64_at(m, &["outputTokenLimit"]),
                supports_tools: None,
                pricing: None,
                source: ModelSource::Listed,
            })
        })
        .collect()
}

#[async_trait]
impl AiProvider for Gemini {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: ID.into(),
            display_name: "Google Gemini".into(),
            capabilities: ProviderCapabilities { streaming: true, tool_calling: true, structured_output: true, model_listing: true, usage_reporting: true, cost_reporting: false, cancellation: true, context_limits: true },
            key_hint: "Keys from Google AI Studio often start with AIza".into(),
            key_page: "https://aistudio.google.com/app/apikey".into(),
            docs_page: "https://ai.google.dev/gemini-api/docs".into(),
            billing_note: "Some models have a free tier with low limits; beyond it requests are billed to your Google Cloud project by tokens. On the free tier, Google may use your prompts to improve its products (see Google's terms). Google does not report a price per response, so Vixalune shows token counts only.".into(),
            verification_note: "Implemented against the documented generateContent streaming (alt=sse) and models.list (checked against the documentation on 2026-10-20). The x-goog-api-key header and error shapes follow Google's examples. Not yet verified with a real key.".into(),
            extra_settings: vec![],
        }
    }

    fn validate_config(&self, config: &ProviderConfig) -> AiResult<()> {
        validate_config_generic(config)?;
        if let Some(m) = &config.default_model {
            if !model_path_ok(m) {
                return Err(AiError::new(AiErrorKind::Configuration, "Gemini model names contain only letters, digits, dots, dashes and underscores"));
            }
        }
        if !config.extra.is_empty() {
            return Err(AiError::new(AiErrorKind::Configuration, "Gemini has no extra settings"));
        }
        Ok(())
    }

    fn validate_request(&self, req: &ChatRequest) -> AiResult<()> {
        validate_common(req)?;
        if !model_path_ok(&req.model) {
            return Err(AiError::new(AiErrorKind::ModelNotFound, "Gemini model names contain only letters, digits, dots, dashes and underscores"));
        }
        Ok(())
    }

    async fn check_auth(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<()> {
        let resp = send(self.get("/models?pageSize=1", key), key, cancel, timeout, parse_error).await?;
        read_json(resp, key, cancel, timeout, MAX_MODELS_BYTES).await.map(|_| ())
    }

    async fn list_models(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<Vec<ModelInfo>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let path = match &token {
                Some(t) if t.len() < 400 && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=' | '.')) => format!("/models?pageSize=1000&pageToken={t}"),
                _ => "/models?pageSize=1000".to_string(),
            };
            let resp = send(self.get(&path, key), key, cancel, timeout, parse_error).await?;
            let v = read_json(resp, key, cancel, timeout, MAX_MODELS_BYTES).await?;
            out.extend(parse_models(&v));
            token = str_at(&v, &["nextPageToken"]).filter(|t| !t.is_empty()).map(String::from);
            if token.is_none() {
                break;
            }
        }
        if out.is_empty() {
            return Err(AiError::new(AiErrorKind::Protocol, "the provider returned no models that can generate content"));
        }
        Ok(out)
    }

    async fn chat_stream(&self, key: &Secret, req: &ChatRequest, _config: &ProviderConfig, sink: &dyn StreamSink, cancel: &CancellationToken) -> AiResult<ChatOutcome> {
        self.validate_request(req)?;
        let rb = self.client.post(format!("{}/models/{}:streamGenerateContent?alt=sse", self.base, req.model)).header("x-goog-api-key", key.expose()).json(&build_body(req));
        let limit = Duration::from_secs(u64::from(req.idle_timeout_secs));
        let resp = send(rb, key, cancel, limit, parse_error).await?;
        let mut em = Emitter::new(sink);
        let mut parser = Parser::default();
        let end = stream_sse(resp, key, cancel, limit, |ev| parser.handle(&ev.data, &mut em)).await.map_err(|e| e.scrubbed(key))?;
        // This API has no terminal event: the stream simply ends after the chunk that carries a finish reason.
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
        ChatRequest { model: "gemini-test".into(), system: Some("be brief".into()), messages: vec![ChatMessage::user("hi"), ChatMessage::assistant("yo"), ChatMessage::user("more")], tools: vec![], params: GenerationParams { temperature: Some(0.2), max_output_tokens: Some(50) }, idle_timeout_secs: 30 }
    }

    #[test]
    fn the_body_maps_roles_system_and_generation_config() {
        let b = build_body(&req());
        assert_eq!(b["contents"][1], json!({"role": "model", "parts": [{"text": "yo"}]}));
        assert_eq!(b["systemInstruction"], json!({"parts": [{"text": "be brief"}]}));
        assert_eq!(b["generationConfig"]["maxOutputTokens"], 50);
        assert!(!b.to_string().contains("key"), "no key-like field in the body");
    }

    #[test]
    fn chunks_with_text_finish_and_usage_are_assembled() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        for d in [
            r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"Hel"}]}}],"modelVersion":"gemini-test"}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":"lo"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"thoughtsTokenCount":3,"totalTokenCount":10}}"#,
        ] {
            assert_eq!(p.handle(d, &mut em).unwrap(), Step::Continue);
        }
        let o = em.into_outcome();
        assert_eq!(o.text, "Hello");
        assert_eq!(o.finish, Some(FinishReason::Stop));
        assert_eq!((o.usage.input_tokens, o.usage.output_tokens, o.usage.total_tokens), (Some(5), Some(5), Some(10)));
        assert_eq!(o.model.as_deref(), Some("gemini-test"));
    }

    #[test]
    fn thoughts_blocked_prompts_function_calls_and_safety_finishes() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        p.handle(r#"{"candidates":[{"content":{"parts":[{"text":"secret thinking","thought":true},{"text":"answer"}]}}]}"#, &mut em).unwrap();
        assert_eq!(em.outcome.text, "answer");
        let e = Parser::default().handle(r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#, &mut Emitter::new(&sink)).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::ContentFiltered);
        let mut em = Emitter::new(&sink);
        let mut p = Parser::default();
        p.handle(r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"lookup","args":{"q":1}}}]},"finishReason":"STOP"}]}"#, &mut em).unwrap();
        let o = em.into_outcome();
        assert_eq!((o.tool_calls[0].name.as_str(), o.tool_calls[0].arguments.as_str()), ("lookup", "{\"q\":1}"));
        assert_eq!(o.finish, Some(FinishReason::ToolCalls));
        assert_eq!(finish_reason("SAFETY"), FinishReason::ContentFilter);
        assert_eq!(finish_reason("MAX_TOKENS"), FinishReason::Length);
    }

    #[test]
    fn google_error_shapes_are_classified_including_the_bad_key_that_arrives_as_400() {
        let bad_key = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT","details":[{"reason":"API_KEY_INVALID"}]}}"#;
        assert_eq!(parse_error(400, bad_key).0, AiErrorKind::Authentication);
        assert_eq!(parse_error(403, r#"{"error":{"code":403,"message":"x","status":"PERMISSION_DENIED"}}"#).0, AiErrorKind::Permission);
        assert_eq!(parse_error(404, r#"{"error":{"code":404,"message":"models/x is not found","status":"NOT_FOUND"}}"#).0, AiErrorKind::ModelNotFound);
        assert_eq!(parse_error(429, r#"{"error":{"code":429,"message":"Resource has been exhausted (e.g. check quota).","status":"RESOURCE_EXHAUSTED"}}"#).0, AiErrorKind::RateLimit);
        assert_eq!(parse_error(429, r#"{"error":{"code":429,"message":"You exceeded your current quota, please check your plan and billing details.","status":"RESOURCE_EXHAUSTED"}}"#).0, AiErrorKind::Quota);
        assert_eq!(parse_error(503, r#"{"error":{"code":503,"message":"The model is overloaded.","status":"UNAVAILABLE"}}"#).0, AiErrorKind::Overloaded);
        assert_eq!(parse_error(400, r#"{"error":{"code":400,"message":"bad field","status":"INVALID_ARGUMENT"}}"#).0, AiErrorKind::InvalidRequest);
        let sink = VecSink::default();
        let e = Parser::default().handle(r#"{"error":{"code":503,"message":"overloaded","status":"UNAVAILABLE"}}"#, &mut Emitter::new(&sink)).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Overloaded);
    }

    #[test]
    fn the_model_list_keeps_models_that_can_generate_and_reads_their_limits() {
        let v = json!({"models":[
            {"name":"models/gemini-a","displayName":"Gemini A","inputTokenLimit":1000000,"outputTokenLimit":8192,"supportedGenerationMethods":["generateContent","countTokens"]},
            {"name":"models/embed-x","supportedGenerationMethods":["embedContent"]},
            {"name":"models/bad id","supportedGenerationMethods":["generateContent"]}
        ]});
        let m = parse_models(&v);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].id.as_str(), m[0].context_window, m[0].max_output_tokens), ("gemini-a", Some(1_000_000), Some(8192)));
    }

    #[test]
    fn model_names_that_could_alter_the_url_are_refused_before_anything_is_sent() {
        let p = Gemini::new();
        for bad in ["a/b", "a:b", "a?x=1", "..", "a b", "x#y"] {
            let mut r = req();
            r.model = bad.into();
            assert!(p.validate_request(&r).is_err(), "{bad}");
        }
        assert!(p.validate_request(&req()).is_ok());
    }
}
