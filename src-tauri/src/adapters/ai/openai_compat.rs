//! The "chat completions" wire format that OpenRouter (and many other gateways) speak: request body, streaming chunks, usage.
//! Kept apart from any one provider so another compatible provider can be registered with a few lines.

use super::common::{error_message, f64_at, parse_json, str_at, u64_at, Emitter};
use super::http::{kind_from_status, Step};
use crate::domain::ai::{AiError, AiErrorKind, AiResult, ChatRequest, FinishReason, Role, StreamEvent, Usage};
use serde_json::{json, Map, Value};

pub fn build_body(req: &ChatRequest, extra: Map<String, Value>) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(s) = &req.system {
        messages.push(json!({"role": "system", "content": s}));
    }
    for m in &req.messages {
        let role = match m.role {
            Role::Assistant => "assistant",
            _ => "user",
        };
        messages.push(json!({"role": role, "content": m.content}));
    }
    let mut body = json!({"model": req.model, "messages": messages, "stream": true});
    let o = body.as_object_mut().unwrap();
    if let Some(t) = req.params.temperature {
        o.insert("temperature".into(), json!(t));
    }
    if let Some(n) = req.params.max_output_tokens {
        o.insert("max_tokens".into(), json!(n));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req.tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.parameters}})).collect();
        o.insert("tools".into(), Value::Array(tools));
    }
    for (k, v) in extra {
        o.insert(k, v);
    }
    body
}

/// Error codes inside a stream or an error body, numeric (HTTP-like) or textual.
pub fn kind_from_code(code: &Value, message: &str) -> AiErrorKind {
    if let Some(n) = code.as_u64() {
        return kind_from_status(n as u16);
    }
    let c = code.as_str().unwrap_or("").to_ascii_lowercase();
    let m = message.to_ascii_lowercase();
    if c.contains("invalid_api_key") || c.contains("authentication") || m.contains("no auth credentials") || m.contains("invalid api key") {
        AiErrorKind::Authentication
    } else if c.contains("insufficient_quota") || c.contains("insufficient_credits") || m.contains("insufficient credits") || m.contains("credit balance") {
        AiErrorKind::Quota
    } else if c.contains("rate_limit") {
        AiErrorKind::RateLimit
    } else if c.contains("model_not_found") || c.contains("not_found") {
        AiErrorKind::ModelNotFound
    } else if c.contains("context_length") {
        AiErrorKind::TooLarge
    } else if c.contains("content_filter") || c.contains("moderation") {
        AiErrorKind::ContentFiltered
    } else if c.contains("server") || c.contains("overloaded") {
        AiErrorKind::Server
    } else {
        AiErrorKind::InvalidRequest
    }
}

/// Error parser for the usual `{"error":{"code","message","type"}}` bodies.
pub fn parse_error_body(status: u16, body: &str) -> (AiErrorKind, String) {
    let msg = error_message(body).unwrap_or_default();
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v.get("error").and_then(|e| e.get("code")).cloned().unwrap_or(Value::Null);
    let by_status = kind_from_status(status);
    // The status is authoritative for the transient and permission classes; the code refines the 4xx cases.
    let kind = match by_status {
        AiErrorKind::RateLimit => {
            if kind_from_code(&code, &msg) == AiErrorKind::Quota {
                AiErrorKind::Quota
            } else {
                AiErrorKind::RateLimit
            }
        }
        AiErrorKind::InvalidRequest => {
            let k = kind_from_code(&code, &msg);
            if matches!(k, AiErrorKind::Authentication | AiErrorKind::Quota | AiErrorKind::ModelNotFound | AiErrorKind::TooLarge | AiErrorKind::ContentFiltered) {
                k
            } else {
                AiErrorKind::InvalidRequest
            }
        }
        other => other,
    };
    (kind, msg)
}

fn finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCalls,
        "content_filter" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

pub fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u64_at(u, &["prompt_tokens"]),
        output_tokens: u64_at(u, &["completion_tokens"]),
        total_tokens: u64_at(u, &["total_tokens"]),
        cost_usd: f64_at(u, &["cost"]),
    }
}

#[derive(Default)]
pub struct ChatCompletionsParser {
    started: bool,
    finished: bool,
}

impl ChatCompletionsParser {
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Handle one `data:` payload. `Step::Done` on `[DONE]`.
    pub fn handle(&mut self, data: &str, em: &mut Emitter<'_>) -> AiResult<Step> {
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(Step::Done);
        }
        let v = parse_json(data)?;
        if let Some(err) = v.get("error") {
            let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("the provider reported an error");
            let kind = kind_from_code(err.get("code").unwrap_or(&Value::Null), message);
            return Err(AiError::new(kind, message));
        }
        if !self.started {
            self.started = true;
            em.emit(StreamEvent::Started { model: str_at(&v, &["model"]).map(String::from) });
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let usage = parse_usage(u);
            if !usage.is_empty() {
                em.emit(StreamEvent::Usage { usage });
            }
        }
        for choice in v.get("choices").and_then(|c| c.as_array()).into_iter().flatten() {
            let delta = choice.get("delta").unwrap_or(&Value::Null);
            if let Some(text) = delta.get("content").and_then(|c| c.as_str()).filter(|t| !t.is_empty()) {
                if !em.emit(StreamEvent::TextDelta { text: text.to_string() }) {
                    self.finished = true;
                    return Ok(Step::Done);
                }
            }
            for call in delta.get("tool_calls").and_then(|c| c.as_array()).into_iter().flatten() {
                let index = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                if let (Some(id), Some(name)) = (str_at(call, &["id"]), str_at(call, &["function", "name"])) {
                    em.emit(StreamEvent::ToolCallStart { index, id: id.to_string(), name: name.to_string() });
                }
                if let Some(frag) = str_at(call, &["function", "arguments"]).filter(|f| !f.is_empty()) {
                    em.emit(StreamEvent::ToolCallArguments { index, fragment: frag.to_string() });
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                if reason == "error" {
                    return Err(AiError::new(AiErrorKind::Server, "the provider ended the response with an error"));
                }
                self.finished = true;
                em.emit(StreamEvent::Finished { reason: finish_reason(reason) });
            }
        }
        Ok(Step::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ai::common::VecSink;
    use crate::domain::ai::{ChatMessage, GenerationParams, ToolDescription};

    fn req() -> ChatRequest {
        ChatRequest { model: "vendor/model".into(), system: Some("be brief".into()), messages: vec![ChatMessage::user("hi"), ChatMessage::assistant("hello"), ChatMessage::user("again")], tools: vec![], params: GenerationParams { temperature: Some(0.5), max_output_tokens: Some(100) }, idle_timeout_secs: 30 }
    }

    #[test]
    fn the_body_has_the_documented_shape_and_no_secrets() {
        let mut r = req();
        r.tools = vec![ToolDescription { name: "lookup".into(), description: "d".into(), parameters: json!({"type": "object"}) }];
        let b = build_body(&r, Map::new());
        assert_eq!(b["model"], "vendor/model");
        assert_eq!(b["stream"], true);
        assert_eq!(b["messages"][0], json!({"role": "system", "content": "be brief"}));
        assert_eq!(b["messages"][2]["role"], "assistant");
        assert_eq!(b["temperature"], 0.5);
        assert_eq!(b["max_tokens"], 100);
        assert_eq!(b["tools"][0]["function"]["name"], "lookup");
    }

    #[test]
    fn a_stream_with_text_usage_cost_and_a_finish_is_assembled() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = ChatCompletionsParser::default();
        for d in [
            r#"{"model":"vendor/model","choices":[{"delta":{"role":"assistant","content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":2,"total_tokens":11,"cost":0.00042}}"#,
        ] {
            assert_eq!(p.handle(d, &mut em).unwrap(), Step::Continue);
        }
        assert_eq!(p.handle("[DONE]", &mut em).unwrap(), Step::Done);
        let o = em.into_outcome();
        assert_eq!(o.text, "Hello");
        assert_eq!(o.finish, Some(FinishReason::Stop));
        assert_eq!((o.usage.input_tokens, o.usage.output_tokens, o.usage.cost_usd), (Some(9), Some(2), Some(0.00042)));
        assert_eq!(o.model.as_deref(), Some("vendor/model"));
    }

    #[test]
    fn tool_calls_stream_in_fragments() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = ChatCompletionsParser::default();
        for d in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"lookup","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"q\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}]}"#,
        ] {
            p.handle(d, &mut em).unwrap();
        }
        let o = em.into_outcome();
        assert_eq!(o.tool_calls.len(), 1);
        assert_eq!((o.tool_calls[0].name.as_str(), o.tool_calls[0].arguments.as_str()), ("lookup", "{\"q\":1}"));
        assert_eq!(o.finish, Some(FinishReason::ToolCalls));
    }

    #[test]
    fn errors_in_the_stream_are_classified_and_malformed_data_is_a_protocol_error() {
        let sink = VecSink::default();
        let mut em = Emitter::new(&sink);
        let mut p = ChatCompletionsParser::default();
        let e = p.handle(r#"{"error":{"code":429,"message":"slow down"}}"#, &mut em).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::RateLimit);
        let e = p.handle(r#"{"error":{"code":"insufficient_quota","message":"out of credit"}}"#, &mut em).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Quota);
        assert_eq!(p.handle("{not json", &mut em).unwrap_err().kind, AiErrorKind::Protocol);
        assert!(p.handle(r#"{"choices":[{"delta":{},"finish_reason":"error"}]}"#, &mut em).is_err());
    }

    #[test]
    fn error_bodies_refine_the_status() {
        assert_eq!(parse_error_body(429, r#"{"error":{"code":"insufficient_quota","message":"x"}}"#).0, AiErrorKind::Quota);
        assert_eq!(parse_error_body(429, r#"{"error":{"message":"x"}}"#).0, AiErrorKind::RateLimit);
        assert_eq!(parse_error_body(400, r#"{"error":{"code":"invalid_api_key","message":"x"}}"#).0, AiErrorKind::Authentication);
        assert_eq!(parse_error_body(401, "plain text").0, AiErrorKind::Authentication);
        assert_eq!(parse_error_body(402, r#"{"error":{"message":"Insufficient credits"}}"#).0, AiErrorKind::Quota);
        assert_eq!(parse_error_body(503, "").0, AiErrorKind::Server);
    }
}
