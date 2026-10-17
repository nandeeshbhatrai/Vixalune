//! Tauri command layer: thin, typed adapters from IPC to application ports. No business logic.

use crate::domain::activity::{ActivityEvent, ActivityKind};
use crate::domain::environment::EnvironmentReport;
use crate::domain::{AppError, AppResult, ToolId};
use crate::ports::{ActivityLogger, CatalogEntry, DependencyResolver, EnvironmentDetector, ToolQuery, ToolRegistry};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

/// Dependency-injection container, built once in `lib.rs::run`.
pub struct AppState {
    pub registry: Arc<dyn ToolRegistry>,
    pub detector: Arc<dyn EnvironmentDetector>,
    pub resolver: Arc<dyn DependencyResolver>,
    pub activity: Arc<dyn ActivityLogger>,
    pub data_dir: PathBuf,
    pub rejected_manifests: Vec<String>,
}

#[derive(Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub data_dir: String,
    pub rejected_manifests: Vec<String>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn log(state: &AppState, kind: ActivityKind, summary: &str) {
    let event = ActivityEvent { at: now(), kind, tool_id: None, summary: summary.to_string() };
    if let Err(e) = state.activity.record(event) {
        tracing::warn!("could not record activity: {e}");
    }
}

#[tauri::command]
pub fn get_app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: "Vixalune",
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.data_dir.display().to_string(),
        rejected_manifests: state.rejected_manifests.clone(),
    }
}

#[tauri::command]
pub async fn detect_environment(state: State<'_, AppState>) -> AppResult<EnvironmentReport> {
    let detector = state.detector.clone();
    let report = tauri::async_runtime::spawn_blocking(move || detector.detect())
        .await
        .map_err(|e| AppError::Environment(e.to_string()))?;
    log(&state, ActivityKind::EnvironmentChecked, &format!("{} setup issue(s) found", report.issues.len()));
    Ok(report)
}

#[tauri::command]
pub fn list_tools(state: State<'_, AppState>, query: Option<ToolQuery>) -> Vec<CatalogEntry> {
    state.registry.list(&query.unwrap_or_default())
}

#[tauri::command]
pub fn get_tool(state: State<'_, AppState>, id: String) -> AppResult<CatalogEntry> {
    state.registry.get(&ToolId::new(id)?)
}

#[tauri::command]
pub fn recent_activity(state: State<'_, AppState>, limit: Option<usize>) -> AppResult<Vec<ActivityEvent>> {
    state.activity.recent(limit.unwrap_or(20).min(200))
}
