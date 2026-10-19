//! Tauri command layer: thin, typed adapters from IPC to application ports. No business logic.

use crate::domain::activity::{ActivityEvent, ActivityKind};
use crate::domain::environment::EnvironmentReport;
use crate::domain::jobs::{Capabilities, InstalledTool, Job, LinkKind, ToolStateEntry};
use crate::domain::lab::{LabState, LabStatus};
use crate::domain::learning::Tutorial;
use crate::domain::runs::{LaunchDefaults, OutputRow, PurgeReport, RunRecord};
use crate::domain::settings::UserSettings;
use crate::domain::storage::{validate_storage_dir, StorageInfo, StorageKind, StorageUpdate};
use crate::domain::LabId;
use crate::adapters::launch_router::LaunchRouter;
use crate::adapters::result_saver::suggested_file_name;
use crate::adapters::run_support::RunSupport;
use crate::domain::container::RunApprovals;
use crate::domain::manifest::ParamValue;
use crate::domain::platform::{PlatformReport, SetupActionId};
use crate::domain::process::InstallPlan;
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::{
    ActivityLogger, CatalogEntry, DependencyResolver, EnvironmentDetector, InstallationManager, JobRegistry, JobStore, LabManager, LearningCatalog, ResultSaver, RunHistory, SettingsProvider,
    LinkOpener, PlatformDetector, RunPreview, SetupManager, ToolQuery, ToolRegistry, ToolRunner,
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
    pub platform: Arc<dyn PlatformDetector>,
    pub setup: Arc<dyn SetupManager>,
    pub resolver: Arc<dyn DependencyResolver>,
    pub installer: Arc<dyn InstallationManager>,
    pub runner: Arc<dyn ToolRunner>,
    pub jobs: Arc<dyn JobRegistry>,
    pub labs: Arc<dyn LabManager>,
    pub launch: Arc<LaunchRouter>,
    pub store: Arc<dyn JobStore>,
    pub history: Arc<dyn RunHistory>,
    pub settings: Arc<dyn SettingsProvider>,
    pub saver: Arc<dyn ResultSaver>,
    pub learning: Arc<dyn LearningCatalog>,
    pub ai: Arc<crate::adapters::ai::service::AiService>,
    pub support: Arc<RunSupport>,
    pub links: Arc<dyn LinkOpener>,
    pub capabilities: Capabilities,
    pub activity: Arc<dyn ActivityLogger>,
    pub data_dir: PathBuf,
    pub rejected_manifests: Vec<String>,
    /// Things that happened at start-up the user should know about (for example a damaged database that was replaced).
    pub startup_notices: Vec<String>,
    pub config_path: PathBuf,
    pub config: std::sync::Mutex<crate::config::AppConfig>,
    pub data_dir_overridden: bool,
    pub restart_required: std::sync::atomic::AtomicBool,
}

#[derive(Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub data_dir: String,
    pub rejected_manifests: Vec<String>,
    pub startup_notices: Vec<String>,
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
        startup_notices: state.startup_notices.clone(),
    }
}

#[tauri::command]
pub async fn detect_environment(state: State<'_, AppState>) -> AppResult<EnvironmentReport> {
    let report = state.detector.detect().await;
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

#[tauri::command]
pub async fn preview_launch(
    state: State<'_, AppState>,
    id: String,
    profile: String,
    values: Option<HashMap<String, ParamValue>>,
    approvals: Option<RunApprovals>,
) -> AppResult<RunPreview> {
    state.runner.preview(&ToolId::new(id)?, &profile, &values.unwrap_or_default(), &approvals.unwrap_or_default()).await
}

/// Start a launch profile. Container runs only start with exactly the approvals the user confirmed.
#[tauri::command]
pub async fn launch_tool(
    state: State<'_, AppState>,
    id: String,
    profile: String,
    values: Option<HashMap<String, ParamValue>>,
    approvals: Option<RunApprovals>,
) -> AppResult<OperationId> {
    state.runner.start(&ToolId::new(id)?, &profile, &values.unwrap_or_default(), &approvals.unwrap_or_default()).await
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


/// Full platform report: facts, readiness per capability, and proposed (never performed) setup steps.
#[tauri::command]
pub async fn get_platform_report(state: State<'_, AppState>) -> AppResult<PlatformReport> {
    let report = state.platform.detect().await;
    log(&state, ActivityKind::EnvironmentChecked, "platform readiness checked");
    Ok(report)
}

/// Run one automatic setup step. `confirmed` must be true: this changes the computer.
#[tauri::command]
pub async fn run_setup_action(state: State<'_, AppState>, id: SetupActionId, confirmed: bool) -> AppResult<OperationId> {
    state.setup.run_action(id, confirmed).await
}

/// Schedule a Windows restart after a cancellable delay. Only after explicit confirmation.
#[tauri::command]
pub async fn request_restart(state: State<'_, AppState>, confirmed: bool, delay_secs: Option<u32>) -> AppResult<()> {
    state.setup.request_restart(confirmed, delay_secs.unwrap_or(60)).await
}

#[tauri::command]
pub async fn cancel_restart(state: State<'_, AppState>) -> AppResult<()> {
    state.setup.cancel_restart().await
}

/// Open the official guide for a setup step. The URL comes from the fixed step definition, not the UI.
#[tauri::command]
pub fn open_setup_link(state: State<'_, AppState>, id: SetupActionId) -> AppResult<()> {
    let link = crate::domain::platform::setup_action(id).official_link.ok_or_else(|| AppError::NotFound("a guide for that step".into()))?;
    state.links.open(&link)
}


// ------------------------------------------------------------------ phase 5: running tools, history, labs, learning

#[tauri::command]
pub fn get_launch_defaults(state: State<'_, AppState>, id: String, profile: String) -> AppResult<LaunchDefaults> {
    state.launch.defaults(&ToolId::new(id)?, &profile)
}

#[tauri::command]
pub fn get_job(state: State<'_, AppState>, id: OperationId) -> AppResult<Job> {
    state.store.get_job(id)
}

/// Output lines with a sequence number greater than `after` (0 = from the start). The UI polls this while a run is active.
#[tauri::command]
pub fn get_run_output(state: State<'_, AppState>, id: OperationId, after: Option<i64>) -> AppResult<Vec<OutputRow>> {
    state.store.output_since(id, after.unwrap_or(0).max(0), 2000)
}

#[tauri::command]
pub fn list_runs(state: State<'_, AppState>, limit: Option<usize>) -> AppResult<Vec<RunRecord>> {
    state.history.list_runs(limit.unwrap_or(50).clamp(1, 500))
}

#[tauri::command]
pub fn get_run(state: State<'_, AppState>, id: OperationId) -> AppResult<RunRecord> {
    state.history.get_run(id)
}

/// Save a run's stored output. The backend opens the Save dialog and writes the file; the UI never supplies a path or the text.
#[tauri::command]
pub async fn save_run_output(state: State<'_, AppState>, id: OperationId) -> AppResult<Option<String>> {
    let run = state.history.get_run(id)?;
    let lines = state.store.job_output(id, 100_000)?;
    if lines.is_empty() {
        return Err(AppError::NotFound("stored output for this run (it may have expired under your retention settings)".into()));
    }
    let name = suggested_file_name(run.tool_id.as_str(), &run.profile_id, &run.started_at, "txt");
    let saved = state.saver.save(&name, &lines.join("\n")).await?;
    if let Some(path) = &saved {
        state.history.set_result_path(id, path)?;
        let _ = state.activity.record(ActivityEvent { at: now(), kind: ActivityKind::ResultSaved, tool_id: Some(run.tool_id.clone()), summary: format!("output of '{}' saved", run.profile_id) });
    }
    Ok(saved)
}

/// Show a run's own folder (where tools such as ZAP write report files) in Explorer.
#[tauri::command]
pub fn open_run_folder(state: State<'_, AppState>, id: OperationId) -> AppResult<()> {
    let run = state.history.get_run(id)?;
    let dir = run.run_dir.ok_or_else(|| AppError::NotFound("a folder for this run".into()))?;
    let dir = PathBuf::from(dir);
    // Only folders this application created for runs can be opened.
    if dir.parent() != Some(state.support.runs_dir.as_path()) {
        return Err(AppError::Validation("that is not a run folder".into()));
    }
    state.links.open_folder(&dir)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> UserSettings {
    state.settings.get()
}

#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, settings: UserSettings) -> AppResult<UserSettings> {
    state.settings.update(settings)?;
    log(&state, ActivityKind::SettingsChanged, "lab list or retention settings changed");
    let _ = state.support.purge_expired();
    Ok(state.settings.get())
}

/// Delete all finished run history and stored output now.
#[tauri::command]
pub fn purge_history_now(state: State<'_, AppState>) -> AppResult<PurgeReport> {
    let cutoff = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let report = state.history.purge(&cutoff, &cutoff)?;
    state.support.remove_stale_run_dirs();
    log(&state, ActivityKind::HistoryPurged, &format!("{} run record(s) deleted", report.runs_deleted));
    Ok(report)
}

#[tauri::command]
pub fn list_labs(state: State<'_, AppState>) -> Vec<LabStatus> {
    state.labs.list().into_iter().map(|lab| {
        let st = state.labs.state(&lab.id).unwrap_or(LabState::Stopped);
        LabStatus { lab, state: st }
    }).collect()
}

#[tauri::command]
pub async fn start_lab(state: State<'_, AppState>, id: String) -> AppResult<Vec<LabStatus>> {
    state.labs.start(&LabId::new(id)?).await?;
    log(&state, ActivityKind::LabStarted, "learning lab started");
    Ok(list_labs(state))
}

#[tauri::command]
pub async fn stop_lab(state: State<'_, AppState>, id: String) -> AppResult<Vec<LabStatus>> {
    state.labs.stop(&LabId::new(id)?).await?;
    log(&state, ActivityKind::LabStopped, "learning lab stopped");
    Ok(list_labs(state))
}

#[tauri::command]
pub fn list_tutorials(state: State<'_, AppState>) -> Vec<Tutorial> {
    state.learning.list()
}

#[tauri::command]
pub fn get_tutorial(state: State<'_, AppState>, id: String) -> AppResult<Tutorial> {
    state.learning.get(&ToolId::new(id)?)
}

/// Open one of a tutorial's official links. The UI names the tutorial and the link's position; the address is read from the validated tutorial.
#[tauri::command]
pub fn open_tutorial_link(state: State<'_, AppState>, tool_id: String, index: usize) -> AppResult<()> {
    let t = state.learning.get(&ToolId::new(tool_id)?)?;
    let link = t.links.get(index).ok_or_else(|| AppError::NotFound("that link".into()))?;
    state.links.open(&link.url)
}

// ------------------------------------------------------------------ phase 7: storage and release information

fn storage_info(state: &AppState) -> StorageInfo {
    let cfg = state.config.lock().unwrap();
    StorageInfo {
        data_dir: state.data_dir.display().to_string(),
        config_path: state.config_path.display().to_string(),
        database_path: cfg.storage.database_path.display().to_string(),
        runs_dir: state.support.runs_dir.display().to_string(),
        tools_dir: cfg.storage.tools_dir.display().to_string(),
        labs_dir: cfg.storage.labs_dir.display().to_string(),
        data_dir_overridden: state.data_dir_overridden,
        restart_required: state.restart_required.load(std::sync::atomic::Ordering::SeqCst),
    }
}

#[tauri::command]
pub fn get_storage_info(state: State<'_, AppState>) -> StorageInfo {
    storage_info(&state)
}

/// Change the tools and labs folders. They are validated, created and proven writable now, saved to config.toml, and used from the next start.
#[tauri::command]
pub fn update_storage(state: State<'_, AppState>, update: StorageUpdate) -> AppResult<StorageInfo> {
    let tools = validate_storage_dir(&update.tools_dir, StorageKind::Tools)?;
    let labs = validate_storage_dir(&update.labs_dir, StorageKind::Labs)?;
    for dir in [&tools, &labs] {
        std::fs::create_dir_all(dir).map_err(|e| AppError::Validation(format!("the folder '{}' cannot be created: {e}", dir.display())))?;
        // Prove it is writable now rather than failing later in the middle of a run.
        let probe = dir.join(format!(".vixalune-write-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::write(&probe, b"x").map_err(|e| AppError::Validation(format!("the folder '{}' is not writable: {e}", dir.display())))?;
        let _ = std::fs::remove_file(&probe);
    }
    {
        let mut cfg = state.config.lock().unwrap();
        let mut next = cfg.clone();
        next.storage.tools_dir = tools;
        next.storage.labs_dir = labs;
        next.save(&state.config_path)?;
        *cfg = next;
    }
    state.restart_required.store(true, std::sync::atomic::Ordering::SeqCst);
    log(&state, ActivityKind::SettingsChanged, "storage locations changed (applied at the next start)");
    Ok(storage_info(&state))
}

/// Open the project's release page. The address is fixed in the backend; Vixalune never downloads or runs updates itself.
#[tauri::command]
pub fn open_release_page(state: State<'_, AppState>) -> AppResult<()> {
    state.links.open(crate::domain::storage::RELEASE_PAGE)
}
