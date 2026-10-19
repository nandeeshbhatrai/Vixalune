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

// ------------------------------------------------------------------ tool-aware assistant

#[derive(serde::Serialize)]
pub struct TurnEntry {
    pub message_id: String,
    pub turn: crate::domain::assistant::model::AssistantTurn,
}

/// Ask the tool-aware assistant. Retrieval, lookups and validation happen in the backend; the model only drafts text that is checked.
#[tauri::command]
pub async fn assistant_ask(app: AppHandle, state: State<'_, AppState>, input: SendInput) -> AppResult<SendAck> {
    let assistant = state.assistant.clone();
    assistant.ask(input, Arc::new(TauriAiEvents(app))).await
}

#[tauri::command]
pub async fn assistant_retry(app: AppHandle, state: State<'_, AppState>, conversation_id: String, provider: String, model: String) -> AppResult<SendAck> {
    let assistant = state.assistant.clone();
    assistant.retry(&conversation_id, provider, model, Arc::new(TauriAiEvents(app))).await
}

/// `consent` must be true: it is the person's answer to "send part of this run's output to your AI provider?".
#[tauri::command]
pub async fn assistant_explain_run(app: AppHandle, state: State<'_, AppState>, conversation_id: String, run_id: String, consent: bool, provider: String, model: String) -> AppResult<SendAck> {
    let assistant = state.assistant.clone();
    assistant.explain_run(&conversation_id, &run_id, consent, provider, model, Arc::new(TauriAiEvents(app))).await
}

#[tauri::command]
pub fn assistant_turns(state: State<'_, AppState>, conversation_id: String) -> AppResult<Vec<TurnEntry>> {
    Ok(state.assistant.turns(&conversation_id)?.into_iter().map(|(message_id, turn)| TurnEntry { message_id, turn }).collect())
}

/// An action the person chose on a card. The backend validates and authorizes it again; installing and running are not actions.
#[tauri::command]
pub async fn assistant_run_action(state: State<'_, AppState>, action: crate::domain::assistant::actions::Action) -> AppResult<crate::adapters::assistant::executor::ActionOutcome> {
    let assistant = state.assistant.clone();
    assistant.run_action(action).await
}

/// Opens the web address of one piece of evidence of a stored answer. The address comes from the stored answer, never from the
/// interface, and only https is opened.
#[tauri::command]
pub fn assistant_open_evidence(state: State<'_, AppState>, conversation_id: String, message_id: String, evidence_id: String) -> AppResult<()> {
    let turn = state.assistant.turns(&conversation_id)?.into_iter().find(|t| t.0 == message_id).map(|t| t.1).ok_or_else(|| crate::domain::AppError::NotFound("that answer".into()))?;
    let url = turn.evidence.iter().find(|e| e.id == evidence_id).and_then(|e| e.url.clone()).ok_or_else(|| crate::domain::AppError::NotFound("that source".into()))?;
    if !url.starts_with("https://") {
        return Err(crate::domain::AppError::Validation("only https addresses are opened".into()));
    }
    state.links.open(&url)
}
