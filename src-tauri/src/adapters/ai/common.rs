//! Helpers every adapter uses: collecting stream events into an outcome (with the size limit applied before anything reaches the
//! UI), request checks that do not depend on the provider, and small JSON accessors.

use crate::domain::ai::{valid_model_id, AiError, AiErrorKind, AiResult, ChatOutcome, ChatRequest, Role, StreamEvent, MAX_CONTEXT_CHARS, MAX_RESPONSE_CHARS};
use crate::ports::StreamSink;
use serde_json::Value;

/// Folds events into a [`ChatOutcome`] and forwards them to the sink. Text beyond the response limit is cut off before it is
/// forwarded, so what the UI shows is what is stored.
pub struct Emitter<'a> {
    pub outcome: ChatOutcome,
    sink: &'a dyn StreamSink,
}

impl<'a> Emitter<'a> {
    pub fn new(sink: &'a dyn StreamSink) -> Self {
        Self { outcome: ChatOutcome::default(), sink }
    }

    /// Returns `false` when the response size limit has been reached and reading should stop.
    pub fn emit(&mut self, ev: StreamEvent) -> bool {
        if let StreamEvent::TextDelta { text } = &ev {
            let have = self.outcome.text.chars().count();
            if have + text.chars().count() > MAX_RESPONSE_CHARS {
                let room = MAX_RESPONSE_CHARS.saturating_sub(have);
                let cut: String = text.chars().take(room).collect();
                if !cut.is_empty() {
                    self.sink.event(&StreamEvent::TextDelta { text: cut });
                }
                self.outcome.apply(&ev);
                return false;
            }
        }
        self.sink.event(&ev);
        self.outcome.apply(&ev)
    }

    pub fn into_outcome(mut self) -> ChatOutcome {
        if self.outcome.truncated_by_limit && self.outcome.finish.is_none() {
            self.outcome.finish = Some(crate::domain::ai::FinishReason::Length);
        }
        self.outcome
    }
}

/// Checks that hold for every provider. Adapters add their own.
pub fn validate_common(req: &ChatRequest) -> AiResult<()> {
    let bad = |m: &str| Err(AiError::new(AiErrorKind::InvalidRequest, m));
    if !valid_model_id(&req.model) {
        return Err(AiError::new(AiErrorKind::ModelNotFound, "choose a valid model name"));
    }
    if req.messages.is_empty() {
        return bad("there is no message to send");
    }
    if req.messages.iter().any(|m| m.role == Role::Tool || !m.tool_calls.is_empty() || m.tool_call_id.is_some()) {
        return Err(AiError::new(AiErrorKind::Unsupported, "conversations that include tool results are not supported yet"));
    }
    if req.messages.last().map(|m| m.role) != Some(Role::User) {
        return bad("the conversation must end with a message from you");
    }
    if req.messages.iter().any(|m| m.role == Role::System) {
        return bad("put instructions in the system field, not in the messages");
    }
    let chars: usize = req.messages.iter().map(|m| m.content.chars().count()).sum::<usize>() + req.system.as_deref().map_or(0, |s| s.chars().count());
    if chars > MAX_CONTEXT_CHARS {
        return Err(AiError::new(AiErrorKind::TooLarge, "the conversation is too long to send; start a new conversation"));
    }
    if req.messages.iter().any(|m| m.content.trim().is_empty()) {
        return bad("a message is empty");
    }
    if req.tools.len() > 32 {
        return bad("too many tools");
    }
    for t in &req.tools {
        if t.name.is_empty() || t.name.len() > 64 || !t.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return bad("a tool name is not valid");
        }
        if !t.parameters.is_object() {
            return bad("a tool's parameters must be a JSON Schema object");
        }
    }
    if let Some(t) = req.params.temperature {
        if !t.is_finite() || t < 0.0 {
            return bad("the temperature is not valid");
        }
    }
    if req.idle_timeout_secs == 0 {
        return bad("the timeout is not valid");
    }
    Ok(())
}

pub fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str()
}

pub fn u64_at(v: &Value, path: &[&str]) -> Option<u64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_u64()
}

pub fn f64_at(v: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_f64()
}

pub fn parse_json(data: &str) -> AiResult<Value> {
    serde_json::from_str(data).map_err(|_| AiError::new(AiErrorKind::Protocol, "the provider sent a response Vixalune could not read"))
}

/// A message from an error body of the usual `{"error": {"message": ...}}` shapes, if there is one.
pub fn error_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let e = v.get("error").unwrap_or(&v);
    e.get("message").and_then(|m| m.as_str()).map(|s| s.to_string()).or_else(|| e.as_str().map(|s| s.to_string()))
}

pub struct VecSink(pub std::sync::Mutex<Vec<StreamEvent>>);

impl Default for VecSink {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl VecSink {
    pub fn events(&self) -> Vec<StreamEvent> {
        self.0.lock().unwrap().clone()
    }
}

impl StreamSink for VecSink {
    fn event(&self, event: &StreamEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}
