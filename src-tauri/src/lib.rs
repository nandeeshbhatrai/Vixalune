//! Vixalune core. Layering (outer -> inner):
//! `commands` (IPC) -> `ports` (traits) <- `adapters` / `engine` (implementations) -> `domain` (pure types).

pub mod adapters;
pub mod commands;
pub mod config;
pub mod domain;
pub mod engine;
pub mod logging;
pub mod ports;

use adapters::{
    activity_sqlite::SqliteActivityLogger,
    env_detector::SystemEnvironmentDetector,
    executor::SystemCommandExecutor,
    link_opener::SystemLinkOpener,
    locator::SystemExecutableLocator,
    registry::ManifestRegistry,
    resolver::BasicDependencyResolver,
    sqlite_store::SqliteJobStore,
    unimplemented::{UnimplementedLabManager, UnimplementedToolRunner},
    winget::WingetAdapter,
};
use commands::AppState;
use config::AppConfig;
use domain::activity::ActivityKind;
use domain::jobs::Capabilities;
use engine::{EngineDeps, InstallEngine};
use ports::{InstallationManager, InstallerAdapter, TrustLevel};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

fn manifests_dir(app: &tauri::App) -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests")
    } else {
        app.path().resource_dir().map(|d| d.join("manifests")).unwrap_or_default()
    }
}

fn build_state(app: &tauri::App) -> Result<AppState, Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let config = AppConfig::load_or_default(&data_dir.join("config.toml"), &data_dir)?;

    let registry = ManifestRegistry::load_dir(&manifests_dir(app), TrustLevel::Bundled)?;
    let rejected: Vec<String> = registry.rejected.iter().map(|e| e.to_string()).collect();
    for r in &rejected {
        tracing::warn!("rejected manifest: {r}");
    }
    let registry = Arc::new(registry);
    let activity = Arc::new(SqliteActivityLogger::open(&config.storage.database_path)?);

    // The only adapter implemented and tested so far is winget; other install methods are
    // reported as unsupported rather than faked.
    let winget: Arc<dyn InstallerAdapter> =
        Arc::new(WingetAdapter::new(Arc::new(SystemCommandExecutor), Arc::new(SystemExecutableLocator)));
    let engine = InstallEngine::new(EngineDeps {
        registry: registry.clone(),
        detector: Arc::new(SystemEnvironmentDetector),
        resolver: Arc::new(BasicDependencyResolver),
        adapters: vec![winget],
        store: Arc::new(SqliteJobStore::open(&config.storage.database_path)?),
        activity: activity.clone(),
    });
    for job in engine.recover()? {
        tracing::warn!("closed interrupted job {} for {}", job.id, job.tool_id);
    }
    let capabilities = Capabilities {
        install: true,
        uninstall: true,
        verify: true,
        launch: false,
        jobs: true,
        labs: false,
        installed_detection: true,
        install_methods: engine.supported_methods(),
    };
    {
        let engine = engine.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = engine.refresh_states().await {
                tracing::warn!("initial tool detection failed: {e}");
            }
        });
    }

    let state = AppState {
        registry,
        detector: Arc::new(SystemEnvironmentDetector),
        resolver: Arc::new(BasicDependencyResolver),
        installer: Arc::new(engine.clone()) as Arc<dyn InstallationManager>,
        runner: Arc::new(UnimplementedToolRunner),
        jobs: Arc::new(engine),
        labs: Arc::new(UnimplementedLabManager),
        links: Arc::new(SystemLinkOpener),
        capabilities,
        activity,
        data_dir,
        rejected_manifests: rejected,
    };
    commands::log(&state, ActivityKind::AppStarted, "Vixalune started");
    Ok(state)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();
    tauri::Builder::default()
        .setup(|app| {
            let state = build_state(app)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::detect_environment,
            commands::list_tools,
            commands::get_tool,
            commands::recent_activity,
            commands::get_capabilities,
            commands::list_tool_states,
            commands::list_installed_tools,
            commands::list_jobs,
            commands::get_job_output,
            commands::cancel_job,
            commands::install_tool,
            commands::uninstall_tool,
            commands::verify_tool,
            commands::launch_tool,
            commands::list_labs,
            commands::open_link,
            commands::get_install_plan,
            commands::approve_job,
            commands::refresh_tool_states,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vixalune");
}
