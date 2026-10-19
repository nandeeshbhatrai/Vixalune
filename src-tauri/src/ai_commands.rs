//! IPC for the AI gateway. Thin: every rule lives in `AiService`. The only place a key crosses this boundary is
//! `ai_save_credential`, inbound; nothing returned to the interface ever contains one.

use crate::adapters::ai::service::{ConversationDetail, ModelList, ProviderView, ProvidersOverview, SendAck, SendInput, TestReport};
use crate::commands::AppState;
use crate::domain::ai::{AiSettings, AiUiEvent, Conversation, ProviderConfig};
use crate::domain::secret::Secret;
use crate::domain::AppResult;
use crate::ports::AiEventSink;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

/// The event name the interface listens to for streamed answers.
pub const AI_EVENT: &str = "ai-stream";

/// Delivers stream events to the window. Events carry text and ids only.
pub struct TauriAiEvents(pub AppHandle);

impl AiEventSink for TauriAiEvents {
    fn emit(&self, event: AiUiEvent) {
        if let Err(e) = self.0.emit(AI_EVENT, event) {
            tracing::warn!("could not deliver an AI event: {e}");
        }
    }
}

#[tauri::command]
pub fn ai_overview(state: State<'_, AppState>) -> AppResult<ProvidersOverview> {
    state.ai.overview()
}

/// The key arrives once, is wrapped immediately (so it is wiped when dropped) and is never echoed back.
#[tauri::command]
pub fn ai_save_credential(state: State<'_, AppState>, provider: String, api_key: String, label: Option<String>) -> AppResult<ProviderView> {
    state.ai.save_credential(&provider, Secret::new(api_key), label)
}

#[tauri::command]
pub fn ai_delete_credential(state: State<'_, AppState>, provider: String) -> AppResult<ProviderView> {
    state.ai.delete_credential(&provider)
}

#[tauri::command]
pub fn ai_update_config(state: State<'_, AppState>, config: ProviderConfig) -> AppResult<ProviderView> {
    state.ai.update_config(config)
}

#[tauri::command]
pub fn ai_set_active(state: State<'_, AppState>, provider: String) -> AppResult<ProviderView> {
    state.ai.set_active(&provider)
}

#[tauri::command]
pub fn ai_update_settings(state: State<'_, AppState>, settings: AiSettings) -> AppResult<AiSettings> {
    state.ai.update_settings(settings)
}

#[tauri::command]
pub async fn ai_list_models(state: State<'_, AppState>, provider: String) -> AppResult<ModelList> {
    state.ai.list_models(&provider).await
}

#[tauri::command]
pub async fn ai_test_connection(state: State<'_, AppState>, provider: String, model: Option<String>) -> AppResult<TestReport> {
    state.ai.test_connection(&provider, model).await
}

#[tauri::command]
pub fn ai_create_conversation(state: State<'_, AppState>, provider: Option<String>, model: Option<String>) -> AppResult<Conversation> {
    state.ai.create_conversation(provider, model)
}

#[tauri::command]
pub fn ai_list_conversations(state: State<'_, AppState>) -> AppResult<Vec<Conversation>> {
    state.ai.list_conversations()
}

#[tauri::command]
pub fn ai_get_conversation(state: State<'_, AppState>, id: String) -> AppResult<ConversationDetail> {
    state.ai.get_conversation(&id)
}

#[tauri::command]
pub fn ai_delete_conversation(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.ai.delete_conversation(&id)
}

#[tauri::command]
pub fn ai_delete_all_conversations(state: State<'_, AppState>) -> AppResult<usize> {
    state.ai.delete_all_conversations()
}

/// Async so the request task is spawned on the async runtime.
#[tauri::command]
pub async fn ai_send_message(app: AppHandle, state: State<'_, AppState>, input: SendInput) -> AppResult<SendAck> {
    state.ai.send_message(input, Arc::new(TauriAiEvents(app)))
}

#[tauri::command]
pub async fn ai_retry(app: AppHandle, state: State<'_, AppState>, conversation_id: String, provider: String, model: String) -> AppResult<SendAck> {
    state.ai.retry_last(&conversation_id, provider, model, Arc::new(TauriAiEvents(app)))
}

#[tauri::command]
pub fn ai_cancel(state: State<'_, AppState>, request_id: String) -> AppResult<()> {
    state.ai.cancel(&request_id)
}

/// Opens a provider's key or documentation page. The address comes from the provider's own descriptor, never from the interface.
#[tauri::command]
pub fn ai_open_provider_page(state: State<'_, AppState>, provider: String, page: String) -> AppResult<()> {
    let overview = state.ai.overview()?;
    let view = overview.providers.iter().find(|p| p.descriptor.id == provider).ok_or_else(|| crate::domain::AppError::Validation("unknown provider".into()))?;
    let url = match page.as_str() {
        "key" => &view.descriptor.key_page,
        "docs" => &view.descriptor.docs_page,
        _ => return Err(crate::domain::AppError::Validation("unknown page".into())),
    };
    state.links.open(url)
}
