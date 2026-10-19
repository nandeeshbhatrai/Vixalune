//! OpenRouter: an OpenAI-compatible gateway to many upstream models. Documented at https://openrouter.ai/docs.
//!
//! Provider-specific behavior kept on purpose: the model list (with context sizes and prices), usage accounting with the
//! provider-reported cost, the `/key` check, the application-title headers, and the `allow_upstream_fallbacks` setting. By
//! default Vixalune asks OpenRouter **not** to fall back to other upstream hosts, so a failure is shown, not hidden.

use super::common::{f64_at, parse_json, str_at, u64_at, validate_common, Emitter};
use super::http::{self, read_json, send, stream_sse, StreamEnd};
use super::openai_compat::{build_body, parse_error_body, ChatCompletionsParser};
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::ports::{AiProvider, StreamSink};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::time::Duration;

pub const ID: &str = "openrouter";
const DEFAULT_BASE: &str = "https://openrouter.ai/api/v1";
const MAX_MODELS_BYTES: usize = 16 * 1024 * 1024;

pub struct OpenRouter {
    base: String,
    client: reqwest::Client,
}

impl OpenRouter {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_BASE).expect("the default address is valid")
    }

    pub fn with_base_url(base: &str) -> AiResult<Self> {
        http::check_base_url(base)?;
        Ok(Self { base: base.trim_end_matches('/').to_string(), client: http::build_client() })
    }

    fn get(&self, path: &str, key: &Secret) -> reqwest::RequestBuilder {
        self.headers(self.client.get(format!("{}{path}", self.base)), key)
    }

    fn headers(&self, rb: reqwest::RequestBuilder, key: &Secret) -> reqwest::RequestBuilder {
        rb.bearer_auth(key.expose()).header("X-Title", "Vixalune").header("HTTP-Referer", "https://github.com/nandeeshbhatrai/Vixalune")
    }
}

impl Default for OpenRouter {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_models(v: &Value) -> Vec<ModelInfo> {
    let per_mtok = |s: Option<&Value>| s.and_then(|x| x.as_str()).and_then(|x| x.parse::<f64>().ok()).map(|p| p * 1_000_000.0);
    v.get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = str_at(m, &["id"])?.to_string();
            if !valid_model_id(&id) {
                return None;
            }
            let pricing = m.get("pricing").map(|p| ModelPricing { input_per_mtok: per_mtok(p.get("prompt")), output_per_mtok: per_mtok(p.get("completion")) });
            let tools = m.get("supported_parameters").and_then(|s| s.as_array()).map(|a| a.iter().any(|x| x.as_str() == Some("tools")));
            Some(ModelInfo {
                id,
                display_name: str_at(m, &["name"]).map(String::from),
                context_window: u64_at(m, &["context_length"]),
                max_output_tokens: u64_at(m, &["top_provider", "max_completion_tokens"]),
                supports_tools: tools,
                pricing,
                source: ModelSource::Listed,
            })
        })
        .collect()
}

#[async_trait]
impl AiProvider for OpenRouter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: ID.into(),
            display_name: "OpenRouter".into(),
            capabilities: ProviderCapabilities { streaming: true, tool_calling: true, structured_output: false, model_listing: true, usage_reporting: true, cost_reporting: true, cancellation: true, context_limits: true },
            key_hint: "Keys start with sk-or-".into(),
            key_page: "https://openrouter.ai/settings/keys".into(),
            docs_page: "https://openrouter.ai/docs".into(),
            billing_note: "Requests use your OpenRouter credits. OpenRouter reports the cost of each response; models marked free may be rate limited. Prices differ by model.".into(),
            verification_note: "Implemented against the documented chat-completions streaming format and /models, /key. Not yet verified with a real key.".into(),
            extra_settings: vec!["allow_upstream_fallbacks".into()],
        }
    }

    fn validate_config(&self, config: &ProviderConfig) -> AiResult<()> {
        validate_config_generic(config)?;
        for (k, v) in &config.extra {
            match (k.as_str(), v.as_str()) {
                ("allow_upstream_fallbacks", "true" | "false") => {}
                ("allow_upstream_fallbacks", _) => return Err(AiError::new(AiErrorKind::Configuration, "allow_upstream_fallbacks must be true or false")),
                _ => return Err(AiError::new(AiErrorKind::Configuration, format!("unknown OpenRouter setting '{}'", sanitize_message(k, None)))),
            }
        }
        Ok(())
    }

    fn validate_request(&self, req: &ChatRequest) -> AiResult<()> {
        validate_common(req)?;
        if req.params.temperature.is_some_and(|t| t > 2.0) {
            return Err(AiError::new(AiErrorKind::InvalidRequest, "the temperature must be between 0 and 2"));
        }
        Ok(())
    }

    async fn check_auth(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<()> {
        // /models can be read without a key, so it proves nothing; /key requires a valid one.
        let resp = send(self.get("/key", key), key, cancel, timeout, parse_error_body).await?;
        read_json(resp, key, cancel, timeout, 64 * 1024).await.map(|_| ())
    }

    async fn list_models(&self, key: &Secret, timeout: Duration, cancel: &CancellationToken) -> AiResult<Vec<ModelInfo>> {
        let resp = send(self.get("/models", key), key, cancel, timeout, parse_error_body).await?;
        let v = read_json(resp, key, cancel, timeout, MAX_MODELS_BYTES).await?;
        let models = parse_models(&v);
        if models.is_empty() {
            return Err(AiError::new(AiErrorKind::Protocol, "the provider returned no models"));
        }
        Ok(models)
    }

    async fn chat_stream(&self, key: &Secret, req: &ChatRequest, config: &ProviderConfig, sink: &dyn StreamSink, cancel: &CancellationToken) -> AiResult<ChatOutcome> {
        self.validate_request(req)?;
        let mut extra = Map::new();
        extra.insert("usage".into(), json!({"include": true}));
        let fallbacks = config.extra.get("allow_upstream_fallbacks").map(|v| v == "true").unwrap_or(false);
        extra.insert("provider".into(), json!({"allow_fallbacks": fallbacks}));
        let body = build_body(req, extra);
        let rb = self.headers(self.client.post(format!("{}/chat/completions", self.base)), key).json(&body);
        let limit = Duration::from_secs(u64::from(req.idle_timeout_secs));
        let resp = send(rb, key, cancel, limit, parse_error_body).await?;
        let mut em = Emitter::new(sink);
        let mut parser = ChatCompletionsParser::default();
        let end = stream_sse(resp, key, cancel, limit, |ev| parser.handle(&ev.data, &mut em)).await.map_err(|e| e.scrubbed(key))?;
        if end == StreamEnd::Closed && !parser.finished() {
            return Err(AiError::new(AiErrorKind::Network, "the connection closed before the response was complete"));
        }
        Ok(em.into_outcome())
    }
}

#[allow(dead_code)]
fn _unused(_: &Value) {
    let _ = (f64_at, parse_json);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_list_is_parsed_with_context_prices_and_tool_support() {
        let v: Value = serde_json::from_str(
            r#"{"data":[
              {"id":"vendor/chat-a","name":"Vendor: Chat A","context_length":128000,"pricing":{"prompt":"0.000002","completion":"0.000006"},"supported_parameters":["temperature","tools"],"top_provider":{"max_completion_tokens":4096}},
              {"id":"vendor/free:free","name":"Free","context_length":8000,"pricing":{"prompt":"0","completion":"0"},"supported_parameters":["temperature"]},
              {"id":"bad id with spaces"},{"name":"no id"}
            ]}"#,
        )
        .unwrap();
        let m = parse_models(&v);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].context_window, Some(128000));
        assert_eq!(m[0].max_output_tokens, Some(4096));
        assert_eq!(m[0].supports_tools, Some(true));
        let p = m[0].pricing.as_ref().unwrap();
        assert!((p.input_per_mtok.unwrap() - 2.0).abs() < 1e-9 && (p.output_per_mtok.unwrap() - 6.0).abs() < 1e-9);
        assert_eq!(m[1].supports_tools, Some(false));
        assert!(m.iter().all(|x| x.source == ModelSource::Listed));
    }

    #[test]
    fn provider_specific_settings_are_validated() {
        let p = OpenRouter::new();
        let mut c = ProviderConfig::new(ID);
        assert!(p.validate_config(&c).is_ok());
        c.extra.insert("allow_upstream_fallbacks".into(), "true".into());
        assert!(p.validate_config(&c).is_ok());
        c.extra.insert("allow_upstream_fallbacks".into(), "maybe".into());
        assert!(p.validate_config(&c).is_err());
        c.extra.clear();
        c.extra.insert("api_key".into(), "sk-or-leak".into());
        assert!(p.validate_config(&c).is_err(), "unknown settings are refused, so a key cannot be smuggled into a setting");
    }

    #[test]
    fn the_descriptor_reports_what_is_and_is_not_verified() {
        let d = OpenRouter::new().descriptor();
        assert!(d.capabilities.cost_reporting && d.capabilities.model_listing);
        assert!(d.verification_note.contains("Not yet verified"));
        assert!(OpenRouter::with_base_url("http://openrouter.example").is_err());
    }
}
