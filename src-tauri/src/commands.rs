//! Tauri command layer: thin, typed adapters from IPC to application ports. No business logic.

use crate::domain::activity::{ActivityEvent, ActivityKind};
use crate::domain::environment::EnvironmentReport;
use crate::domain::jobs::{Capabilities, InstalledTool, Job, LinkKind, ToolStateEntry};
use crate::domain::lab::LabDefinition;
use crate::domain::process::{OutputLine, OutputSink};
use crate::domain::process::InstallPlan;
use crate::domain::{AppError, AppResult, OperationId, RunId, ToolId};
use crate::ports::{
    ActivityLogger, CatalogEntry, DependencyResolver, EnvironmentDetector, InstallationManager, JobRegistry, LabManager,
    LinkOpener, ToolQuery, ToolRegistry, ToolRunner,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

/// Dependency-injection container, built once in `lib.rs::run`.
pub struct AppState {
    pub registry: Arc<dyn ToolRegistry>,
    pub detector: Arc<dyn EnvironmentDetector>,
    pub resolver: Arc<dyn DependencyResolver>,
    pub installer: Arc<dyn InstallationManager>,
    pub runner: Arc<dyn ToolRunner>,
    pub jobs: Arc<dyn JobRegistry>,
    pub labs: Arc<dyn LabManager>,
    pub links: Arc<dyn LinkOpener>,
    pub capabilities: Capabilities,
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

// ------------------------------------------------------------------ phase 2: UI read models and actions

#[tauri::command]
pub fn get_capabilities(state: State<'_, AppState>) -> Capabilities {
    state.capabilities.clone()
}

#[tauri::command]
pub fn list_tool_states(state: State<'_, AppState>) -> AppResult<Vec<ToolStateEntry>> {
    state.installer.states()
}

#[tauri::command]
pub fn list_installed_tools(state: State<'_, AppState>) -> AppResult<Vec<InstalledTool>> {
    state.installer.installed()
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> AppResult<Vec<Job>> {
    state.jobs.list()
}

#[tauri::command]
pub fn get_job_output(state: State<'_, AppState>, id: OperationId) -> AppResult<Vec<String>> {
    state.jobs.output(id)
}

#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>, id: OperationId) -> AppResult<()> {
    state.jobs.cancel(id).await
}

#[tauri::command]
pub async fn install_tool(state: State<'_, AppState>, id: String, allow_elevation: Option<bool>) -> AppResult<OperationId> {
    state.installer.install(&ToolId::new(id)?, allow_elevation.unwrap_or(false)).await
}

#[tauri::command]
pub async fn uninstall_tool(state: State<'_, AppState>, id: String, allow_elevation: Option<bool>) -> AppResult<OperationId> {
    state.installer.uninstall(&ToolId::new(id)?, allow_elevation.unwrap_or(false)).await
}

#[tauri::command]
pub async fn verify_tool(state: State<'_, AppState>, id: String) -> AppResult<OperationId> {
    state.installer.verify(&ToolId::new(id)?).await
}

struct DiscardOutput;
impl OutputSink for DiscardOutput {
    fn line(&self, _: OutputLine) {}
}

/// Launch with default parameters. Output streaming is wired up with the run interface.
#[tauri::command]
pub async fn launch_tool(state: State<'_, AppState>, id: String) -> AppResult<RunId> {
    state.runner.start(&ToolId::new(id)?, "default", &HashMap::new(), Arc::new(DiscardOutput)).await
}

#[tauri::command]
pub fn list_labs(state: State<'_, AppState>) -> Vec<LabDefinition> {
    state.labs.list()
}

/// The UI names a tool and a link kind; the URL is read from the validated manifest.
#[tauri::command]
pub fn open_link(state: State<'_, AppState>, tool_id: String, kind: LinkKind) -> AppResult<()> {
    let entry = state.registry.get(&ToolId::new(tool_id)?)?;
    let m = &entry.manifest;
    let url = match kind {
        LinkKind::Homepage => Some(m.homepage.as_str()),
        LinkKind::Documentation => Some(m.documentation.as_str()),
        LinkKind::SourceRepository => m.source_repository.as_deref(),
    };
    let url = url.ok_or_else(|| AppError::NotFound("that link".into()))?;
    state.links.open(url)
}

#[tauri::command]
pub async fn get_install_plan(state: State<'_, AppState>, id: String) -> AppResult<InstallPlan> {
    state.installer.plan(&ToolId::new(id)?).await
}

/// Grant the consent a job is waiting for (it asked before doing anything that may need elevation).
#[tauri::command]
pub async fn approve_job(state: State<'_, AppState>, id: OperationId) -> AppResult<()> {
    state.installer.approve(id).await
}

/// Re-detect which catalog tools are present and persist the answers.
#[tauri::command]
pub async fn refresh_tool_states(state: State<'_, AppState>) -> AppResult<()> {
    state.installer.refresh_states().await
}
