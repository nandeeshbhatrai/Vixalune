//! The AI gateway: provider adapters, the shared HTTP and streaming plumbing, credential storage, persistence and the service
//! that ties them together. Documentation for users and for adding a provider: docs/AI_GATEWAY.md.

#[cfg(test)]
mod adapter_tests;
pub mod anthropic;
pub mod common;
pub mod credentials;
pub mod gemini;
pub mod http;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
pub mod mock_server;
pub mod openai;
pub mod openai_compat;
pub mod openrouter;
pub mod service;
#[cfg(test)]
mod service_tests;
pub mod sse;
pub mod store;

use crate::domain::ai::{valid_provider_id, AiError, AiErrorKind, AiResult, ProviderDescriptor};
use crate::ports::AiProvider;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The set of providers the application knows. Adding one is a single `register` call; the service, the commands and the
/// interface work from the provider's descriptor and capabilities, never from its name.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, Arc<dyn AiProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// OpenAI, Google Gemini, Anthropic and OpenRouter, against their official addresses.
    pub fn builtin() -> Self {
        let mut r = Self::new();
        for p in [
            Arc::new(openai::OpenAi::new()) as Arc<dyn AiProvider>,
            Arc::new(gemini::Gemini::new()),
            Arc::new(anthropic::Anthropic::new()),
            Arc::new(openrouter::OpenRouter::new()),
        ] {
            r.register(p).expect("built-in providers have unique valid ids");
        }
        r
    }

    pub fn register(&mut self, provider: Arc<dyn AiProvider>) -> AiResult<()> {
        let id = provider.descriptor().id;
        if !valid_provider_id(&id) {
            return Err(AiError::new(AiErrorKind::Configuration, "a provider id may contain only lowercase letters, digits and dashes"));
        }
        if self.providers.contains_key(&id) {
            return Err(AiError::new(AiErrorKind::Configuration, format!("provider '{id}' is already registered")));
        }
        self.providers.insert(id, provider);
        Ok(())
    }

    pub fn get(&self, id: &str) -> AiResult<Arc<dyn AiProvider>> {
        self.providers.get(id).cloned().ok_or_else(|| AiError::new(AiErrorKind::Configuration, "unknown provider"))
    }

    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.providers.values().map(|p| p.descriptor()).collect()
    }
}
