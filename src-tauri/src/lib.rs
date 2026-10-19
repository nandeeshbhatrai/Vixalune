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
    container::ContainerAdapter,
    container_runner::ContainerToolRunner,
    elevation::UacElevatedRunner,
    executor::SystemCommandExecutor,
    host_facts::WindowsHostFacts,
    link_opener::SystemLinkOpener,
    locator::SystemExecutableLocator,
    platform_detector::SystemPlatformDetector,
    registry::ManifestRegistry,
    resolver::BasicDependencyResolver,
    setup::SetupEngine,
    sqlite_store::SqliteJobStore,
    unimplemented::UnimplementedLabManager,
    winget::WingetAdapter,
    wsl::WslAptAdapter,
};
use commands::AppState;
use config::AppConfig;
use domain::activity::ActivityKind;
use domain::jobs::Capabilities;
use engine::{EngineDeps, InstallEngine, RoutedJobs};
use ports::{EnvironmentDetector, InstallationManager, InstallerAdapter, PlatformDetector, SetupManager, ToolRunner, TrustLevel};
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
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);

    let registry = ManifestRegistry::load_dir(&manifests_dir(app), TrustLevel::Bundled)?;
    let rejected: Vec<String> = registry.rejected.iter().map(|e| e.to_string()).collect();
    for r in &rejected {
        tracing::warn!("rejected manifest: {r}");
    }
    let registry = Arc::new(registry);
    let activity = Arc::new(SqliteActivityLogger::open(&config.storage.database_path)?);
    let store = Arc::new(SqliteJobStore::open(&config.storage.database_path)?);

    let executor = Arc::new(SystemCommandExecutor);
    let locator = Arc::new(SystemExecutableLocator);
    let platform = Arc::new(SystemPlatformDetector::new(
        executor.clone(),
        locator.clone(),
        Arc::new(WindowsHostFacts),
        config.storage.tools_dir.clone(),
        config.runtime.wsl_distribution.clone(),
    ));

    // Adapters. Each is an explicit, audited integration; there is no generic "run this" adapter.
    let container = Arc::new(ContainerAdapter::new(executor.clone(), locator.clone(), config.runtime.approved_registries.clone(), home.clone()));
    let adapters: Vec<Arc<dyn InstallerAdapter>> = vec![
        Arc::new(WingetAdapter::new(executor.clone(), locator.clone())),
        Arc::new(WslAptAdapter::new(executor.clone(), locator.clone(), config.runtime.wsl_distribution.clone())),
        container.clone(),
    ];
    let engine = InstallEngine::new(EngineDeps {
        registry: registry.clone(),
        detector: platform.clone() as Arc<dyn EnvironmentDetector>,
        resolver: Arc::new(BasicDependencyResolver),
        adapters,
        store: store.clone(),
        activity: activity.clone(),
    });
    for job in engine.recover()? {
        tracing::warn!("closed interrupted job {} for {}", job.id, job.tool_id);
    }
    let runner = Arc::new(ContainerToolRunner::new(
        registry.clone(),
        executor.clone(),
        locator.clone(),
        container,
        store.clone(),
        activity.clone(),
        config.runtime.approved_registries.clone(),
        home,
    ));
    let setup = SetupEngine::new(platform.clone() as Arc<dyn PlatformDetector>, executor, Arc::new(UacElevatedRunner), locator, store.clone(), activity.clone());

    let capabilities = Capabilities {
        install: true,
        uninstall: true,
        verify: true,
        launch: true,
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
        detector: platform.clone() as Arc<dyn EnvironmentDetector>,
        platform: platform as Arc<dyn PlatformDetector>,
        setup: Arc::new(setup) as Arc<dyn SetupManager>,
        resolver: Arc::new(BasicDependencyResolver),
        installer: Arc::new(engine.clone()) as Arc<dyn InstallationManager>,
        runner: runner.clone() as Arc<dyn ToolRunner>,
        jobs: Arc::new(RoutedJobs { engine, runner }),
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
            commands::get_platform_report,
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
            commands::preview_launch,
            commands::launch_tool,
            commands::run_setup_action,
            commands::request_restart,
            commands::cancel_restart,
            commands::open_setup_link,
            commands::list_labs,
            commands::open_link,
            commands::get_install_plan,
            commands::approve_job,
            commands::refresh_tool_states,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vixalune");
}

