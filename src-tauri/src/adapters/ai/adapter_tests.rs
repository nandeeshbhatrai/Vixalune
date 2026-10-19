//! The same behavioral suite run against every provider adapter, over real HTTP to a local mock server. Nothing here contacts a
//! real provider and no real key is needed.

use super::common::VecSink;
use super::mock_server::{Captured, MockServer, Reply};
use super::{anthropic::Anthropic, gemini::Gemini, openai::OpenAi, openrouter::OpenRouter};
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::ports::AiProvider;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

const KEY: &str = "sk-test-0123456789abcdefghij";

struct Case {
    name: &'static str,
    make: fn(&str) -> Arc<dyn AiProvider>,
    model: &'static str,
    key_header: &'static str,
    key_value: fn(&str) -> String,
    text_event: fn(&str) -> String,
    /// A complete successful stream whose text is "Hello" and which reports input and output tokens.
    success: fn() -> Vec<String>,
    /// A stream that ends cleanly after some text but never says it finished.
    no_finish: fn() -> Vec<String>,
    auth_failure: fn(&str) -> Reply,
    rate_limit: fn() -> Reply,
    server_error: fn() -> Reply,
    models_ok: fn() -> Reply,
    models_expect: &'static str,
}

fn req(model: &str, idle: u32) -> ChatRequest {
    ChatRequest { model: model.into(), system: Some("be brief".into()), messages: vec![ChatMessage::user("hi")], tools: vec![], params: GenerationParams { temperature: None, max_output_tokens: Some(64) }, idle_timeout_secs: idle }
}

fn cfg(p: &str) -> ProviderConfig {
    ProviderConfig::new(p)
}

fn key() -> Secret {
    Secret::new(KEY.into())
}

fn s(v: &[String]) -> Vec<&str> {
    v.iter().map(|x| x.as_str()).collect()
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "openai",
            make: |b| Arc::new(OpenAi::with_base_url(&format!("{b}/v1")).unwrap()),
            model: "model-a",
            key_header: "authorization",
            key_value: |k| format!("Bearer {k}"),
            text_event: |t| format!("data: {}\n\n", json!({"type": "response.output_text.delta", "delta": t})),
            success: || {
                vec![
                    format!("data: {}\n\n", json!({"type": "response.created", "response": {"model": "model-a"}})),
                    format!("data: {}\n\n", json!({"type": "response.output_text.delta", "delta": "Hel"})),
                    format!("data: {}\n\n", json!({"type": "response.output_text.delta", "delta": "lo"})),
                    format!("data: {}\n\n", json!({"type": "response.completed", "response": {"usage": {"input_tokens": 7, "output_tokens": 2, "total_tokens": 9}}})),
                ]
            },
            no_finish: || vec![format!("data: {}\n\n", json!({"type": "response.output_text.delta", "delta": "Hel"}))],
            auth_failure: |k| Reply::json(401, &json!({"error": {"code": "invalid_api_key", "message": format!("Incorrect API key provided: {k}. You can find your API key at https://platform.openai.com/account/api-keys.")}}).to_string()),
            rate_limit: || Reply::json(429, r#"{"error":{"code":"rate_limit_exceeded","message":"Rate limit reached"}}"#).header("Retry-After", "7"),
            server_error: || Reply::json(500, r#"{"error":{"message":"The server had an error"}}"#),
            models_ok: || Reply::json(200, r#"{"data":[{"id":"model-a"},{"id":"text-embedding-x"},{"id":"model-b"}]}"#),
            models_expect: "model-a",
        },
        Case {
            name: "anthropic",
            make: |b| Arc::new(Anthropic::with_base_url(b).unwrap()),
            model: "model-a",
            key_header: "x-api-key",
            key_value: |k| k.to_string(),
            text_event: |t| format!("event: content_block_delta\ndata: {}\n\n", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": t}})),
            success: || {
                vec![
                    format!("event: message_start\ndata: {}\n\n", json!({"type": "message_start", "message": {"model": "model-a", "usage": {"input_tokens": 7, "output_tokens": 1}}})),
                    "event: ping\ndata: {\"type\":\"ping\"}\n\n".to_string(),
                    format!("event: content_block_delta\ndata: {}\n\n", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hel"}})),
                    format!("event: content_block_delta\ndata: {}\n\n", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "lo"}})),
                    format!("event: message_delta\ndata: {}\n\n", json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 2}})),
                    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string(),
                ]
            },
            no_finish: || vec![format!("event: content_block_delta\ndata: {}\n\n", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hel"}}))],
            auth_failure: |k| Reply::json(401, &json!({"type": "error", "error": {"type": "authentication_error", "message": format!("invalid x-api-key: {k}")}}).to_string()),
            rate_limit: || Reply::json(429, r#"{"type":"error","error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit"}}"#).header("retry-after", "7"),
            server_error: || Reply::json(500, r#"{"type":"error","error":{"type":"api_error","message":"Internal server error"}}"#),
            models_ok: || Reply::json(200, r#"{"data":[{"id":"model-a","display_name":"Model A"},{"id":"model-b"}],"has_more":false}"#),
            models_expect: "model-a",
        },
        Case {
            name: "gemini",
            make: |b| Arc::new(Gemini::with_base_url(&format!("{b}/v1beta")).unwrap()),
            model: "model-a",
            key_header: "x-goog-api-key",
            key_value: |k| k.to_string(),
            text_event: |t| format!("data: {}\n\n", json!({"candidates": [{"content": {"parts": [{"text": t}]}}]})),
            success: || {
                vec![
                    format!("data: {}\n\n", json!({"candidates": [{"content": {"parts": [{"text": "Hel"}]}}]})),
                    format!("data: {}\n\n", json!({"candidates": [{"content": {"parts": [{"text": "lo"}]}, "finishReason": "STOP"}], "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 2, "totalTokenCount": 9}})),
                ]
            },
            no_finish: || vec![format!("data: {}\n\n", json!({"candidates": [{"content": {"parts": [{"text": "Hel"}]}}]}))],
            auth_failure: |k| Reply::json(400, &json!({"error": {"code": 400, "message": format!("API key not valid. Please pass a valid API key. (key={k})"), "status": "INVALID_ARGUMENT", "details": [{"reason": "API_KEY_INVALID"}]}}).to_string()),
            rate_limit: || Reply::json(429, r#"{"error":{"code":429,"message":"Resource has been exhausted (e.g. check quota).","status":"RESOURCE_EXHAUSTED"}}"#).header("Retry-After", "7"),
            server_error: || Reply::json(500, r#"{"error":{"code":500,"message":"Internal error","status":"INTERNAL"}}"#),
            models_ok: || Reply::json(200, r#"{"models":[{"name":"models/model-a","displayName":"A","inputTokenLimit":1000,"supportedGenerationMethods":["generateContent"]},{"name":"models/embed","supportedGenerationMethods":["embedContent"]}]}"#),
            models_expect: "model-a",
        },
        Case {
            name: "openrouter",
            make: |b| Arc::new(OpenRouter::with_base_url(&format!("{b}/api/v1")).unwrap()),
            model: "vendor/model-a",
            key_header: "authorization",
            key_value: |k| format!("Bearer {k}"),
            text_event: |t| format!("data: {}\n\n", json!({"choices": [{"delta": {"content": t}}]})),
            success: || {
                vec![
                    ": OPENROUTER PROCESSING\n\n".to_string(),
                    format!("data: {}\n\n", json!({"model": "vendor/model-a", "choices": [{"delta": {"content": "Hel"}}]})),
                    format!("data: {}\n\n", json!({"choices": [{"delta": {"content": "lo"}}]})),
                    format!("data: {}\n\n", json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})),
                    format!("data: {}\n\n", json!({"choices": [], "usage": {"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9, "cost": 0.0003}})),
                    "data: [DONE]\n\n".to_string(),
                ]
            },
            no_finish: || vec![format!("data: {}\n\n", json!({"choices": [{"delta": {"content": "Hel"}}]}))],
            auth_failure: |k| Reply::json(401, &json!({"error": {"code": 401, "message": format!("No auth credentials found (token {k})")}}).to_string()),
            rate_limit: || Reply::json(429, r#"{"error":{"code":429,"message":"Rate limit exceeded"}}"#).header("Retry-After", "7"),
            server_error: || Reply::json(502, r#"{"error":{"code":502,"message":"Provider returned error"}}"#),
            models_ok: || Reply::json(200, r#"{"data":[{"id":"vendor/model-a","name":"A","context_length":8000,"pricing":{"prompt":"0.000001","completion":"0.000002"}}]}"#),
            models_expect: "vendor/model-a",
        },
    ]
}

async fn server_with(chat: impl Fn() -> Reply + Send + Sync + 'static, models: impl Fn() -> Reply + Send + Sync + 'static) -> MockServer {
    MockServer::start(move |c: &Captured| if c.method == "POST" { chat() } else { models() }).await
}

async fn chat(p: &dyn AiProvider, model: &str, idle: u32, cancel: &CancellationToken) -> (AiResult<ChatOutcome>, VecSink) {
    let sink = VecSink::default();
    let r = p.chat_stream(&key(), &req(model, idle), &cfg(&p.descriptor().id), &sink, cancel).await;
    (r, sink)
}

async fn run_suite(c: &Case) {
    let none = CancellationToken::new();
    let n = c.name;

    // --- a complete answer, and the key only in its header
    {
        let s = server_with({ let f = c.success; move || Reply::sse(&s_owned(f())) }, || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let (r, sink) = chat(p.as_ref(), c.model, 10, &none).await;
        let o = r.unwrap_or_else(|e| panic!("{n}: {e}"));
        assert_eq!(o.text, "Hello", "{n}");
        assert!(matches!(o.finish, Some(FinishReason::Stop)), "{n}: {:?}", o.finish);
        assert_eq!((o.usage.input_tokens, o.usage.output_tokens), (Some(7), Some(2)), "{n}");
        assert_eq!(sink.events().iter().filter(|e| matches!(e, StreamEvent::TextDelta { .. })).count(), 2, "{n}: deltas reach the sink as they arrive");
        let seen = s.captured();
        assert_eq!(seen.len(), 1, "{n}");
        let r0 = &seen[0];
        assert_eq!(r0.method, "POST", "{n}");
        assert_eq!(r0.header(c.key_header), Some((c.key_value)(KEY).as_str()), "{n}: the key is sent in {}", c.key_header);
        assert!(!r0.body.contains(KEY) && !r0.target.contains(KEY), "{n}: the key must not appear in the URL or the body");
        assert!(r0.header("user-agent").unwrap_or("").starts_with("Vixalune/"), "{n}");
        let other_headers: Vec<_> = r0.headers.iter().filter(|(k, v)| v.contains(KEY) && k.as_str() != c.key_header).collect();
        assert!(other_headers.is_empty(), "{n}: the key appears in another header: {other_headers:?}");
        let body = r0.json();
        assert_eq!(body.get("stream").and_then(|v| v.as_bool()).or(Some(true)), Some(true), "{n}");
    }

    // --- authentication failure; the provider echoes the key and it must not survive
    {
        let f = c.auth_failure;
        let s = server_with(move || f(KEY), move || f(KEY)).await;
        let p = (c.make)(&s.base());
        let (r, _) = chat(p.as_ref(), c.model, 10, &none).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Authentication, "{n}: {e}");
        assert!(!e.message.contains(KEY) && !e.message.contains("0123456789abcdef"), "{n}: leaked: {}", e.message);
        assert!(!e.retryable(), "{n}");
        assert_eq!(s.captured().len(), 1, "{n}: the adapter itself never retries");
        let e = p.check_auth(&key(), Duration::from_secs(5), &none).await.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Authentication, "{n}: check_auth");
        assert!(!e.message.contains(KEY), "{n}");
    }

    // --- rate limit and server error are transient and say when to retry
    {
        let f = c.rate_limit;
        let s = server_with(f, f).await;
        let p = (c.make)(&s.base());
        let e = chat(p.as_ref(), c.model, 10, &none).await.0.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::RateLimit, "{n}: {e}");
        assert_eq!(e.retry_after_secs, Some(7), "{n}");
        assert!(e.retryable(), "{n}");
        let f = c.server_error;
        let s = server_with(f, f).await;
        let p = (c.make)(&s.base());
        let e = chat(p.as_ref(), c.model, 10, &none).await.0.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Server, "{n}: {e}");
        assert!(e.retryable(), "{n}");
    }

    // --- nothing listening: a network error that does not contain the key
    {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let p = (c.make)(&base);
        let e = chat(p.as_ref(), c.model, 5, &none).await.0.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Network, "{n}: {e}");
        assert!(!e.message.contains(KEY), "{n}");
    }

    // --- the provider goes quiet: idle timeout, after the text that did arrive
    {
        let f = c.text_event;
        let s = server_with(move || Reply::sse(&[]).delayed_chunks(vec![(Duration::ZERO, f("Hel").into_bytes()), (Duration::from_secs(4), b": late\n\n".to_vec())]), || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let started = Instant::now();
        let (r, sink) = chat(p.as_ref(), c.model, 1, &none).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Timeout, "{n}: {e}");
        assert!(started.elapsed() < Duration::from_secs(3), "{n}: gave up in time");
        assert!(sink.events().iter().any(|e| matches!(e, StreamEvent::TextDelta { text } if text == "Hel")), "{n}: text received before the pause is not lost");
    }

    // --- cancellation mid-stream returns promptly with a cancelled error
    {
        let f = c.text_event;
        let s = server_with(move || Reply::sse(&[]).delayed_chunks((0..100).map(|_| (Duration::from_millis(100), f("x").into_bytes())).collect()), || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(350)).await;
            c2.cancel();
        });
        let started = Instant::now();
        let (r, sink) = chat(p.as_ref(), c.model, 30, &cancel).await;
        assert_eq!(r.unwrap_err().kind, AiErrorKind::Cancelled, "{n}");
        assert!(started.elapsed() < Duration::from_secs(2), "{n}: stopped promptly");
        assert!(!sink.events().is_empty(), "{n}: some text was delivered before the stop");
        // Cancelled before sending: nothing reaches the server.
        let before = s.captured().len();
        let pre = CancellationToken::new();
        pre.cancel();
        assert_eq!(chat(p.as_ref(), c.model, 30, &pre).await.0.unwrap_err().kind, AiErrorKind::Cancelled, "{n}");
        assert_eq!(s.captured().len(), before, "{n}");
    }

    // --- a connection that breaks mid-response, and a stream that ends without saying it finished
    {
        let f = c.text_event;
        let s = server_with(move || Reply::sse(&[]).delayed_chunks(vec![(Duration::ZERO, f("Hel").into_bytes())]).broken(), || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let (r, sink) = chat(p.as_ref(), c.model, 5, &none).await;
        let e = r.unwrap_err();
        assert!(matches!(e.kind, AiErrorKind::Network | AiErrorKind::Timeout), "{n}: {e}");
        assert!(!sink.events().is_empty(), "{n}: the partial text was delivered");
        let nf = c.no_finish;
        let s = server_with(move || Reply::sse(&s_owned(nf())), || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let e = chat(p.as_ref(), c.model, 5, &none).await.0.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Network, "{n}: a response that never finished is not a success: {e}");
    }

    // --- garbage from the provider is a protocol error, not a crash
    {
        let s = server_with(|| Reply::sse(&["data: {this is not json\n\n"]), || Reply::json(200, "not json")).await;
        let p = (c.make)(&s.base());
        assert_eq!(chat(p.as_ref(), c.model, 5, &none).await.0.unwrap_err().kind, AiErrorKind::Protocol, "{n}");
        let e = p.list_models(&key(), Duration::from_secs(5), &none).await.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Protocol, "{n}: {e}");
    }

    // --- a redirect is never followed (it could carry the key to another host)
    {
        let s = server_with(|| Reply { status: 302, headers: vec![("Location".into(), "http://127.0.0.1:1/steal".into())], chunks: vec![], break_connection: false }, || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let e = chat(p.as_ref(), c.model, 5, &none).await.0.unwrap_err();
        assert!(!matches!(e.kind, AiErrorKind::Cancelled), "{n}");
        assert_eq!(s.captured().len(), 1, "{n}");
    }

    // --- the response size limit
    {
        let f = c.text_event;
        let big = "y".repeat(50_000);
        let s = server_with(move || Reply::sse(&s_owned((0..6).map(|_| f(&big)).collect())), || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let (r, sink) = chat(p.as_ref(), c.model, 10, &none).await;
        let o = r.unwrap_or_else(|e| panic!("{n}: {e}"));
        assert_eq!(o.text.chars().count(), MAX_RESPONSE_CHARS, "{n}");
        assert_eq!(o.finish, Some(FinishReason::Length), "{n}");
        let forwarded: usize = sink.events().iter().map(|e| if let StreamEvent::TextDelta { text } = e { text.chars().count() } else { 0 }).sum();
        assert!(forwarded <= MAX_RESPONSE_CHARS, "{n}: nothing beyond the limit reaches the interface ({forwarded})");
    }

    // --- model list and the cheap authenticated check
    {
        let f = c.models_ok;
        let s = server_with(|| Reply::json(500, "{}"), f).await;
        let p = (c.make)(&s.base());
        let models = p.list_models(&key(), Duration::from_secs(5), &none).await.unwrap_or_else(|e| panic!("{n}: {e}"));
        assert!(models.iter().any(|m| m.id == c.models_expect), "{n}: {models:?}");
        assert!(models.iter().all(|m| m.source == ModelSource::Listed && valid_model_id(&m.id)), "{n}");
        assert!(!models.iter().any(|m| m.id.contains("embed")), "{n}: only models that can chat are offered");
        p.check_auth(&key(), Duration::from_secs(5), &none).await.unwrap_or_else(|e| panic!("{n}: {e}"));
        for r in s.captured() {
            assert_eq!(r.method, "GET", "{n}");
            assert!(!r.target.contains(KEY), "{n}: the key is not in the address");
        }
    }

    // --- requests the provider cannot handle are refused before anything is sent
    {
        let s = server_with(|| Reply::json(200, "{}"), || Reply::json(200, "{}")).await;
        let p = (c.make)(&s.base());
        let mut r = req(c.model, 5);
        r.messages.push(ChatMessage { role: Role::Tool, content: "result".into(), tool_calls: vec![], tool_call_id: Some("1".into()) });
        let sink = VecSink::default();
        let e = p.chat_stream(&key(), &r, &cfg(&p.descriptor().id), &sink, &none).await.unwrap_err();
        assert_eq!(e.kind, AiErrorKind::Unsupported, "{n}");
        let mut r = req(c.model, 5);
        r.model = "not a model!".into();
        assert_eq!(p.chat_stream(&key(), &r, &cfg(&p.descriptor().id), &sink, &none).await.unwrap_err().kind, AiErrorKind::ModelNotFound, "{n}");
        assert!(s.captured().is_empty(), "{n}: nothing was sent");
    }
}

fn s_owned(v: Vec<String>) -> Vec<&'static str> {
    // The mock keeps its bodies for the life of the test process; leaking a few KiB per case keeps the helper signatures simple.
    s(&v).into_iter().map(|x| &*Box::leak(x.to_string().into_boxed_str())).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn openai_adapter_behaves_over_real_http() {
    run_suite(&cases()[0]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn anthropic_adapter_behaves_over_real_http() {
    run_suite(&cases()[1]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gemini_adapter_behaves_over_real_http() {
    run_suite(&cases()[2]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn openrouter_adapter_behaves_over_real_http() {
    run_suite(&cases()[3]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_specific_request_shapes_are_what_the_documentation_describes() {
    // OpenRouter: usage accounting on, upstream fallbacks off by default, titles sent.
    let s = MockServer::start(|_| Reply::sse(&["data: [DONE]\n\n"])).await;
    let p = OpenRouter::with_base_url(&format!("{}/api/v1", s.base())).unwrap();
    let _ = p.chat_stream(&key(), &req("vendor/m", 5), &cfg("openrouter"), &VecSink::default(), &CancellationToken::new()).await;
    let r = &s.captured()[0];
    assert_eq!(r.path(), "/api/v1/chat/completions");
    assert_eq!(r.json()["usage"], json!({"include": true}));
    assert_eq!(r.json()["provider"], json!({"allow_fallbacks": false}), "no silent switching between upstream hosts by default");
    assert_eq!(r.header("x-title"), Some("Vixalune"));
    let mut c = cfg("openrouter");
    c.extra.insert("allow_upstream_fallbacks".into(), "true".into());
    let _ = p.chat_stream(&key(), &req("vendor/m", 5), &c, &VecSink::default(), &CancellationToken::new()).await;
    assert_eq!(s.captured()[1].json()["provider"], json!({"allow_fallbacks": true}));
    assert!(s.captured()[1].json().get("models").is_none(), "Vixalune never asks OpenRouter to try other models");

    // OpenAI: Responses API path, store off.
    let s = MockServer::start(|_| Reply::sse(&["data: {\"type\":\"response.completed\",\"response\":{}}\n\n"])).await;
    let p = OpenAi::with_base_url(&format!("{}/v1", s.base())).unwrap();
    p.chat_stream(&key(), &req("m", 5), &cfg("openai"), &VecSink::default(), &CancellationToken::new()).await.unwrap();
    let r = &s.captured()[0];
    assert_eq!(r.path(), "/v1/responses");
    assert_eq!(r.json()["store"], false);

    // Anthropic: messages path, version header, max_tokens present.
    let s = MockServer::start(|_| Reply::sse(&["data: {\"type\":\"message_stop\"}\n\n"])).await;
    let p = Anthropic::with_base_url(&s.base()).unwrap();
    p.chat_stream(&key(), &req("m", 5), &cfg("anthropic"), &VecSink::default(), &CancellationToken::new()).await.unwrap();
    let r = &s.captured()[0];
    assert_eq!(r.path(), "/v1/messages");
    assert_eq!(r.header("anthropic-version"), Some("2023-06-01"));
    assert!(r.json()["max_tokens"].as_u64().is_some());

    // Gemini: streamGenerateContent with alt=sse, key in a header, model in the path.
    let s = MockServer::start(|_| Reply::sse(&["data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]},\"finishReason\":\"STOP\"}]}\n\n"])).await;
    let p = Gemini::with_base_url(&format!("{}/v1beta", s.base())).unwrap();
    p.chat_stream(&key(), &req("model-a", 5), &cfg("gemini"), &VecSink::default(), &CancellationToken::new()).await.unwrap();
    let r = &s.captured()[0];
    assert_eq!(r.target, "/v1beta/models/model-a:streamGenerateContent?alt=sse");
    assert!(r.header("x-goog-api-key").is_some());
}
