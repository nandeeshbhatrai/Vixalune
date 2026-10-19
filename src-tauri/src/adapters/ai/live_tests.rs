//! Opt-in tests against the real operating system and the real providers. They are `#[ignore]`d, so `cargo test` (and CI)
//! never runs them and never needs a key. Run them on purpose:
//!
//!   cargo test --lib live_ -- --ignored --nocapture
//!
//! Provider tests read the key from the environment (VIXALUNE_TEST_OPENAI_KEY, VIXALUNE_TEST_ANTHROPIC_KEY,
//! VIXALUNE_TEST_GEMINI_KEY, VIXALUNE_TEST_OPENROUTER_KEY), and the model from VIXALUNE_TEST_<NAME>_MODEL. A provider whose
//! variable is not set is skipped, and says so. Each test makes one tiny request; real providers may bill for it.

use super::common::VecSink;
use crate::domain::ai::*;
use crate::domain::process::CancellationToken;
use crate::domain::secret::Secret;
use crate::ports::{AiProvider, CredentialStore};
use std::time::Duration;

/// Writes, reads, replaces and deletes a throwaway credential under a test-only service name, so no real key is touched.
#[cfg(windows)]
#[test]
#[ignore = "uses the real Windows Credential Manager"]
fn live_windows_credential_manager_round_trip() {
    use super::credentials::WindowsCredentialStore;
    let service = format!("app.vixalune.test.{}", uuid::Uuid::new_v4().simple());
    let store = WindowsCredentialStore::with_service(&service);
    let id = "openai";
    assert!(store.get(id).unwrap().is_none());
    assert!(!store.exists(id).unwrap());
    store.set(id, &Secret::new("sk-test-not-a-real-key-0001".into())).unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().expose(), "sk-test-not-a-real-key-0001");
    store.set(id, &Secret::new("sk-test-not-a-real-key-0002".into())).unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().expose(), "sk-test-not-a-real-key-0002");
    assert!(store.exists(id).unwrap());
    assert!(store.delete(id).unwrap());
    assert!(!store.delete(id).unwrap(), "deleting an absent credential is not an error");
    assert!(store.get(id).unwrap().is_none());
    assert!(store.get("Not A Provider").is_err(), "names are validated before they reach the OS");
}

fn live_key(name: &str) -> Option<Secret> {
    match std::env::var(format!("VIXALUNE_TEST_{name}_KEY")) {
        Ok(k) if !k.trim().is_empty() => Some(Secret::new(k.trim().to_string())),
        _ => {
            println!("SKIPPED {name}: VIXALUNE_TEST_{name}_KEY is not set");
            None
        }
    }
}

async fn live_provider(p: &dyn AiProvider, name: &str) {
    let Some(key) = live_key(name) else { return };
    let cancel = CancellationToken::new();
    let t = Duration::from_secs(30);
    p.check_auth(&key, t, &cancel).await.expect("the key was rejected");
    let models = if p.descriptor().capabilities.model_listing { p.list_models(&key, t, &cancel).await.expect("model listing failed") } else { vec![] };
    println!("{name}: {} models listed", models.len());
    let Ok(model) = std::env::var(format!("VIXALUNE_TEST_{name}_MODEL")) else {
        println!("{name}: key accepted; set VIXALUNE_TEST_{name}_MODEL to also test a streamed answer");
        return;
    };
    let req = ChatRequest {
        model,
        system: None,
        messages: vec![ChatMessage::user("Reply with the single word: pong")],
        tools: vec![],
        params: GenerationParams { max_output_tokens: Some(32), ..Default::default() },
        idle_timeout_secs: 30,
    };
    let sink = VecSink::default();
    let out = p.chat_stream(&key, &req, &ProviderConfig::new(&p.descriptor().id), &sink, &cancel).await.expect("the streamed request failed");
    assert!(!out.text.trim().is_empty(), "no text came back");
    assert!(!sink.events().is_empty());
    println!("{name}: answered {:?}, usage {:?}", out.text.trim(), out.usage);
}

#[tokio::test]
#[ignore = "calls the real OpenAI API with VIXALUNE_TEST_OPENAI_KEY"]
async fn live_openai() {
    live_provider(&super::openai::OpenAi::new(), "OPENAI").await;
}

#[tokio::test]
#[ignore = "calls the real Anthropic API with VIXALUNE_TEST_ANTHROPIC_KEY"]
async fn live_anthropic() {
    live_provider(&super::anthropic::Anthropic::new(), "ANTHROPIC").await;
}

#[tokio::test]
#[ignore = "calls the real Gemini API with VIXALUNE_TEST_GEMINI_KEY"]
async fn live_gemini() {
    live_provider(&super::gemini::Gemini::new(), "GEMINI").await;
}

#[tokio::test]
#[ignore = "calls the real OpenRouter API with VIXALUNE_TEST_OPENROUTER_KEY"]
async fn live_openrouter() {
    live_provider(&super::openrouter::OpenRouter::new(), "OPENROUTER").await;
}
