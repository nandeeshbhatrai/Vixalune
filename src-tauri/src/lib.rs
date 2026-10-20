//! Vixalune core. Layering (outer -> inner):
//! `commands` (IPC) -> `ports` (traits) <- `adapters` / `engine` (implementations) -> `domain` (pure types).

pub mod adapters;
pub mod ai_commands;
pub mod discover_commands;
pub mod commands;
pub mod config;
pub mod domain;
pub mod engine;
pub mod logging;
pub mod ports;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod release_tests;

use adapters::{
    activity_sqlite::SqliteActivityLogger,
    container::ContainerAdapter,
    container_runner::ContainerToolRunner,
    elevation::UacElevatedRunner,
    executor::SystemCommandExecutor,
    host_facts::WindowsHostFacts,
    launch_router::LaunchRouter,
    learning::LearningRegistry,
    link_opener::SystemLinkOpener,
    locator::SystemExecutableLocator,
    platform_detector::SystemPlatformDetector,
    registry::ManifestRegistry,
    resolver::BasicDependencyResolver,
    setup::SetupEngine,
    sqlite_store::SqliteJobStore,
    native_runner::NativeToolRunner,
    practice_lab::{PracticeLabs, DEFAULT_PORT},
    result_saver::DialogSaver,
    run_support::RunSupport,
    settings_store::FileSettings,
    system_net::{HttpRedirectProbe, SystemNameResolver},
    winget::WingetAdapter,
    wsl::WslAptAdapter,
};
use commands::AppState;
use config::AppConfig;
use domain::activity::ActivityKind;
use domain::jobs::Capabilities;
use engine::{EngineDeps, InstallEngine, RoutedJobs};
use ports::{EnvironmentDetector, InstallationManager, InstallerAdapter, LabManager, LearningCatalog, PlatformDetector, ResultSaver, RunHistory, SettingsProvider, SetupManager, ToolRunner, TrustLevel};
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
    // The data folder is separate from the program files. VIXALUNE_DATA_DIR chooses another one (portable or test installs).
    let override_dir = domain::storage::data_dir_override(std::env::var_os(domain::storage::DATA_DIR_ENV))?;
    let data_dir_overridden = override_dir.is_some();
    let data_dir = match override_dir {
        Some(d) => d,
        None => app.path().app_data_dir()?,
    };
    std::fs::create_dir_all(&data_dir)?;
    let mut startup_notices: Vec<String> = Vec::new();
    let config_path = data_dir.join("config.toml");
    let (config, moved_config, config_problem) = AppConfig::load_or_recover(&config_path, &data_dir);
    if let Some(why) = config_problem {
        tracing::error!("config.toml was invalid and defaults are used: {why}");
        startup_notices.push(match moved_config {
            Some(p) => format!("config.toml could not be used ({why}). It was kept as {} and default settings are in effect.", p.display()),
            None => format!("config.toml could not be used ({why}); default settings are in effect."),
        });
    }
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);

    let registry = ManifestRegistry::load_dir_or_empty(&manifests_dir(app), TrustLevel::Bundled);
    let rejected: Vec<String> = registry.rejected.iter().map(|e| e.to_string()).collect();
    for r in &rejected {
        tracing::warn!("rejected manifest: {r}");
    }
    // Packages the person added from Discover. Loaded after the reviewed catalog; reviewed ids can never be replaced.
    let community_dir = data_dir.join("community");
    for problem in adapters::community::CommunityCatalog::load_dir(&community_dir, &registry) {
        tracing::warn!("community entry skipped: {problem}");
        startup_notices.push(format!("A community catalog entry could not be used and was skipped: {problem}"));
    }
    let registry = Arc::new(registry);
    // A damaged database is moved aside and replaced, never allowed to stop the application from starting.
    use adapters::sqlite_store::DatabaseCheck;
    match adapters::sqlite_store::check_database(&config.storage.database_path) {
        Ok(DatabaseCheck::Damaged { moved_to }) => {
            tracing::error!("the database was damaged and was moved to {}", moved_to.display());
            startup_notices.push(format!("The local database was damaged. It was moved to {} and a new one was created; run history and the record of installed tools were reset (installed tools are detected again automatically).", moved_to.display()));
        }
        Ok(DatabaseCheck::Newer { moved_to, found }) => {
            tracing::error!("the database has layout version {found}, newer than this build supports; moved to {}", moved_to.display());
            startup_notices.push(format!("The local database was written by a newer version of Vixalune (layout {found}). It was kept untouched as {} and this version started with an empty one. Install the newer version again and rename the file back to restore your history.", moved_to.display()));
        }
        Ok(DatabaseCheck::Healthy { upgraded_from: Some(old), backup }) => {
            tracing::info!("database layout upgraded from {old}");
            if let Some(b) = backup {
                startup_notices.push(format!("Your data was upgraded to this version. A backup of the previous database was saved as {}.", b.display()));
            }
        }
        Ok(_) => {}
        Err(e) => return Err(format!("the database could not be checked or replaced: {e}").into()),
    }
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
    let community = Arc::new(adapters::community::CommunityCatalog::new(
        community_dir,
        registry.clone(),
        Arc::new(adapters::discovery::WingetDiscovery::new(executor.clone(), locator.clone())),
        {
            let engine = engine.clone();
            Arc::new(move |id: &domain::ToolId| engine.state(id).map(|s| !matches!(s, domain::install_state::InstallState::NotInstalled | domain::install_state::InstallState::Cancelled | domain::install_state::InstallState::Failed)).unwrap_or(false))
        },
    ));
    for job in engine.recover()? {
        tracing::warn!("closed interrupted job {} for {}", job.id, job.tool_id);
    }
    // Learning labs, user settings and everything a run needs besides the tool itself.
    let labs = Arc::new(PracticeLabs::new(&config.storage.labs_dir, DEFAULT_PORT));
    if let Err(e) = labs.ensure_sample_project() {
        tracing::warn!("could not create the sample project: {e}");
    }
    let settings = Arc::new(FileSettings::open(data_dir.join("settings.json")));
    let support = Arc::new(RunSupport {
        settings: settings.clone() as Arc<dyn SettingsProvider>,
        labs: labs.clone(),
        history: store.clone() as Arc<dyn RunHistory>,
        resolver: Arc::new(SystemNameResolver),
        probe: Arc::new(HttpRedirectProbe),
        runs_dir: data_dir.join("runs"),
    });
    let closed = support.close_unfinished_runs();
    if closed > 0 {
        tracing::warn!("{closed} run(s) were cut short by the last shutdown and are shown as failed");
    }
    match support.purge_expired() {
        Ok(r) if r.runs_deleted + r.output_lines_deleted + r.jobs_deleted > 0 => tracing::info!("retention purge removed {} run(s), {} output line(s)", r.runs_deleted, r.output_lines_deleted),
        Ok(_) => {}
        Err(e) => tracing::warn!("retention purge failed: {e}"),
    }
    support.remove_stale_run_dirs();

    let container_runner = Arc::new(
        ContainerToolRunner::new(
            registry.clone(),
            executor.clone(),
            locator.clone(),
            container,
            store.clone(),
            activity.clone(),
            config.runtime.approved_registries.clone(),
            home.clone(),
        )
        .with_support(support.clone()),
    );
    let native_runner = Arc::new(NativeToolRunner::new(registry.clone(), executor.clone(), locator.clone(), store.clone(), activity.clone(), support.clone(), home));
    let runner = Arc::new(LaunchRouter::new(registry.clone(), container_runner, native_runner, support.clone(), usize::from(config.resources.max_concurrent_processes)));
    let learning = LearningRegistry::bundled(registry.as_ref());
    for r in &learning.rejected {
        tracing::warn!("rejected tutorial: {r}");
    }
    let setup = SetupEngine::new(platform.clone() as Arc<dyn PlatformDetector>, executor, Arc::new(UacElevatedRunner), locator, store.clone(), activity.clone());

    let capabilities = Capabilities {
        install: true,
        uninstall: true,
        verify: true,
        launch: true,
        jobs: true,
        labs: true,
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

    let ai = {
        let ai_store = Arc::new(adapters::ai::store::SqliteAiStore::open(&config.storage.database_path)?);
        let svc = adapters::ai::service::AiService::new(Arc::new(adapters::ai::ProviderRegistry::builtin()), adapters::ai::credentials::default_store(), ai_store);
        match svc.startup() {
            Ok((closed, purged)) => tracing::info!("AI history: {closed} cut-off reply(ies) closed, {purged} expired conversation(s) removed"),
            Err(e) => tracing::warn!("AI history maintenance failed: {e}"),
        }
        svc
    };
    let assistant = adapters::assistant::service::AssistantService::new(
        ai.clone(),
        Arc::new(adapters::assistant::facts::AppToolFacts {
            registry: registry.clone() as Arc<dyn ports::ToolRegistry>,
            installer: Arc::new(engine.clone()) as Arc<dyn InstallationManager>,
            runner: runner.clone() as Arc<dyn ToolRunner>,
            history: store.clone() as Arc<dyn RunHistory>,
            store: store.clone() as Arc<dyn ports::JobStore>,
        }),
        Arc::new(adapters::assistant::osv::OsvSource::default()),
    );

    let state = AppState {
        registry,
        detector: platform.clone() as Arc<dyn EnvironmentDetector>,
        platform: platform as Arc<dyn PlatformDetector>,
        setup: Arc::new(setup) as Arc<dyn SetupManager>,
        resolver: Arc::new(BasicDependencyResolver),
        installer: Arc::new(engine.clone()) as Arc<dyn InstallationManager>,
        runner: runner.clone() as Arc<dyn ToolRunner>,
        jobs: Arc::new(RoutedJobs { engine, runner: runner.clone() }),
        labs: labs as Arc<dyn LabManager>,
        launch: runner.clone(),
        store: store.clone() as Arc<dyn ports::JobStore>,
        history: store as Arc<dyn RunHistory>,
        settings: settings as Arc<dyn SettingsProvider>,
        saver: Arc::new(DialogSaver::new(app.handle().clone())) as Arc<dyn ResultSaver>,
        learning: Arc::new(learning) as Arc<dyn LearningCatalog>,
        support,
        links: Arc::new(SystemLinkOpener),
        capabilities,
        activity,
        data_dir,
        rejected_manifests: rejected,
        startup_notices,
        ai,
        assistant,
        community,
        config_path,
        config: std::sync::Mutex::new(config.clone()),
        data_dir_overridden,
        restart_required: std::sync::atomic::AtomicBool::new(false),
    };
    commands::log(&state, ActivityKind::AppStarted, &format!("Vixalune {} started", env!("CARGO_PKG_VERSION")));
    Ok(state)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            commands::start_lab,
            commands::stop_lab,
            commands::get_launch_defaults,
            commands::get_job,
            commands::get_run_output,
            commands::list_runs,
            commands::get_run,
            commands::save_run_output,
            commands::open_run_folder,
            commands::get_settings,
            commands::update_settings,
            commands::purge_history_now,
            commands::list_tutorials,
            commands::get_tutorial,
            commands::open_tutorial_link,
            commands::open_link,
            commands::get_storage_info,
            commands::update_storage,
            commands::open_release_page,
            commands::get_install_plan,
            commands::approve_job,
            commands::refresh_tool_states,
            ai_commands::ai_overview,
            ai_commands::ai_save_credential,
            ai_commands::ai_delete_credential,
            ai_commands::ai_update_config,
            ai_commands::ai_set_active,
            ai_commands::ai_update_settings,
            ai_commands::ai_list_models,
            ai_commands::ai_test_connection,
            ai_commands::ai_create_conversation,
            ai_commands::ai_list_conversations,
            ai_commands::ai_get_conversation,
            ai_commands::ai_delete_conversation,
            ai_commands::ai_delete_all_conversations,
            ai_commands::ai_send_message,
            ai_commands::ai_retry,
            ai_commands::ai_cancel,
            ai_commands::ai_open_provider_page,
            ai_commands::assistant_ask,
            ai_commands::assistant_retry,
            ai_commands::assistant_explain_run,
            ai_commands::assistant_turns,
            ai_commands::assistant_run_action,
            ai_commands::assistant_open_evidence,
            discover_commands::discover_search,
            discover_commands::discover_details,
            discover_commands::discover_add,
            discover_commands::community_remove,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Vixalune")
        .run(|handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Stop running tools (and remove their containers) before the process goes away.
                if let Some(state) = handle.try_state::<AppState>() {
                    let stopped = state.ai.cancel_all();
                    if stopped > 0 {
                        tracing::info!("stopped {stopped} AI request(s) at shutdown");
                    }
                    let launch = state.launch.clone();
                    let asked = tauri::async_runtime::block_on(launch.shutdown(std::time::Duration::from_secs(8)));
                    if asked > 0 {
                        tracing::info!("stopped {asked} running tool(s) at shutdown");
                    }
                }
            }
        });
}

