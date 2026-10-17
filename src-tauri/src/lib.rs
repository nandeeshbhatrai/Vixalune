//! Vixalune core. Layering (outer -> inner):
//! `commands` (IPC) -> `ports` (traits) <- `adapters` (implementations) -> `domain` (pure types).

pub mod adapters;
pub mod commands;
pub mod config;
pub mod domain;
pub mod logging;
pub mod ports;

use adapters::{activity_sqlite::SqliteActivityLogger, env_detector::SystemEnvironmentDetector, registry::ManifestRegistry, resolver::BasicDependencyResolver};
use commands::AppState;
use config::AppConfig;
use domain::activity::ActivityKind;
use ports::TrustLevel;
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

    let state = AppState {
        registry: Arc::new(registry),
        detector: Arc::new(SystemEnvironmentDetector),
        resolver: Arc::new(BasicDependencyResolver),
        activity: Arc::new(SqliteActivityLogger::open(&config.storage.database_path)?),
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vixalune");
}
