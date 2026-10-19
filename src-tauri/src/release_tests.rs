//! Tests for upgrade, rollback and data-separation behavior that a packaged release depends on.

use crate::adapters::sqlite_store::{check_database, DatabaseCheck, SqliteJobStore, SCHEMA_VERSION};
use crate::domain::jobs::{Job, JobKind, JobPhase};
use crate::domain::storage::RELEASE_PAGE;
use crate::domain::{OperationId, ToolId};
use crate::ports::JobStore;
use rusqlite::Connection;

fn job() -> Job {
    Job {
        id: OperationId::new(),
        tool_id: ToolId::new("nmap").unwrap(),
        kind: JobKind::Install,
        phase: JobPhase::Succeeded,
        message: "done".into(),
        percent: None,
        started_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        finished_at: Some("2026-01-01T00:00:01Z".into()),
        exit_code: Some(0),
        allow_elevation: false,
        error: None,
    }
}

fn user_version(path: &std::path::Path) -> i64 {
    Connection::open(path).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
}

#[test]
fn a_new_database_is_stamped_with_the_current_layout_version() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("vixalune.db");
    assert_eq!(check_database(&db).unwrap(), DatabaseCheck::Fresh);
    drop(SqliteJobStore::open(&db).unwrap());
    assert_eq!(user_version(&db), SCHEMA_VERSION);
    assert_eq!(check_database(&db).unwrap(), DatabaseCheck::Healthy { upgraded_from: None, backup: None });
}

#[test]
fn an_older_database_is_backed_up_once_then_upgraded_and_keeps_its_data() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("vixalune.db");
    let j = job();
    {
        // A database from before layout versions existed: tables and data, user_version 0.
        let s = SqliteJobStore::open(&db).unwrap();
        s.create_job(&j).unwrap();
    }
    Connection::open(&db).unwrap().execute_batch("PRAGMA user_version = 0;").unwrap();
    let DatabaseCheck::Healthy { upgraded_from, backup } = check_database(&db).unwrap() else { panic!("expected healthy") };
    assert_eq!(upgraded_from, Some(0));
    let backup = backup.expect("a backup is made before the upgrade");
    assert!(backup.to_string_lossy().ends_with(".bak-v0"));
    assert_eq!(user_version(&db), SCHEMA_VERSION);
    // The backup is a complete, openable copy of the old data; the upgraded database kept it too.
    let old = SqliteJobStore::open(&backup).unwrap();
    assert_eq!(old.get_job(j.id).unwrap().id, j.id);
    assert_eq!(SqliteJobStore::open(&db).unwrap().get_job(j.id).unwrap().id, j.id);
    // A second check is a no-op and never overwrites the first backup.
    std::fs::write(&backup, b"first backup must be kept as it is").unwrap();
    Connection::open(&db).unwrap().execute_batch("PRAGMA user_version = 0;").unwrap();
    let DatabaseCheck::Healthy { backup: second, .. } = check_database(&db).unwrap() else { panic!("expected healthy") };
    assert!(second.is_none());
    assert_eq!(std::fs::read(&backup).unwrap(), b"first backup must be kept as it is");
}

#[test]
fn a_database_from_a_newer_version_is_kept_untouched_and_a_fresh_one_is_used() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("vixalune.db");
    let j = job();
    {
        let s = SqliteJobStore::open(&db).unwrap();
        s.create_job(&j).unwrap();
    }
    Connection::open(&db).unwrap().execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 5)).unwrap();
    let DatabaseCheck::Newer { moved_to, found } = check_database(&db).unwrap() else { panic!("expected newer") };
    assert_eq!(found, SCHEMA_VERSION + 5);
    assert!(!db.exists());
    // The newer data is intact and can be moved back by the user or the newer version.
    assert_eq!(user_version(&moved_to), SCHEMA_VERSION + 5);
    assert_eq!(SqliteJobStore::open(&moved_to).unwrap().get_job(j.id).unwrap().id, j.id);
    let fresh = SqliteJobStore::open(&db).unwrap();
    assert!(fresh.list_jobs(5).unwrap().is_empty());
}

#[test]
fn program_files_and_user_data_are_separate_places() {
    // The data folder default comes from the identifier, never from the install folder; the override must be a real folder.
    use crate::domain::storage::{data_dir_override, DATA_DIR_ENV};
    assert_eq!(DATA_DIR_ENV, "VIXALUNE_DATA_DIR");
    let cfg = crate::config::AppConfig::defaults_for(std::path::Path::new("D:\\Data\\Vixalune"));
    assert!(cfg.storage.database_path.starts_with("D:\\Data\\Vixalune"));
    assert!(cfg.storage.tools_dir.starts_with("D:\\Data\\Vixalune"));
    assert!(data_dir_override(Some("C:\\Program Files\\Vixalune\\data".into())).is_err(), "never inside Program Files");
}

#[test]
fn the_release_page_is_a_fixed_https_address_on_github() {
    assert!(RELEASE_PAGE.starts_with("https://github.com/"));
    assert!(url::Url::parse(RELEASE_PAGE).is_ok());
}

#[test]
fn the_packaged_version_numbers_agree() {
    // tauri.conf.json, package.json and Cargo.toml must carry the same version or the installer and the About page disagree.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let conf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("tauri.conf.json")).unwrap()).unwrap();
    let pkg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("..").join("package.json")).unwrap()).unwrap();
    assert_eq!(conf["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(pkg["version"], env!("CARGO_PKG_VERSION"));
    let changelog = std::fs::read_to_string(root.join("..").join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains(&format!("## {} ", env!("CARGO_PKG_VERSION"))), "CHANGELOG.md needs an entry for this version");
}

#[test]
fn the_installer_is_configured_per_user_without_bundled_third_party_tools() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let conf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("tauri.conf.json")).unwrap()).unwrap();
    assert_eq!(conf["bundle"]["active"], true);
    assert_eq!(conf["bundle"]["targets"], serde_json::json!(["nsis"]));
    assert_eq!(conf["bundle"]["windows"]["nsis"]["installMode"], "currentUser", "no administrator rights to install");
    assert_eq!(conf["bundle"]["windows"]["allowDowngrades"], false);
    // Only Vixalune's own data and notices are bundled: no tool binaries.
    let resources = conf["bundle"]["resources"].as_object().unwrap();
    for (src, _) in resources {
        assert!(["../manifests/", "../THIRD_PARTY_LICENSES.md", "../LICENSE"].contains(&src.as_str()), "unexpected bundled resource {src}");
    }
    // No updater is configured: Vixalune never downloads and runs code by itself.
    assert!(conf.get("plugins").and_then(|p| p.get("updater")).is_none());
}

#[test]
fn runs_cut_short_by_a_crash_are_shown_as_failed_after_the_next_start() {
    use crate::adapters::run_support::RunSupport;
    use crate::adapters::settings_store::FileSettings;
    use crate::adapters::system_net::{HttpRedirectProbe, SystemNameResolver};
    use crate::domain::manifest::{NetworkActivity, OutputFormat};
    use crate::domain::runs::{RunOutcome, RunRecord};
    use crate::ports::RunHistory;
    use std::sync::Arc;
    struct NoLabs;
    impl crate::ports::LabInfo for NoLabs {
        fn placeholders(&self) -> std::collections::HashMap<String, String> {
            Default::default()
        }
    }
    let store = Arc::new(SqliteJobStore::in_memory().unwrap());
    let support = RunSupport {
        settings: Arc::new(FileSettings::in_memory()),
        labs: Arc::new(NoLabs),
        history: store.clone(),
        resolver: Arc::new(SystemNameResolver),
        probe: Arc::new(HttpRedirectProbe),
        runs_dir: std::env::temp_dir().join("vixalune-unused"),
    };
    let record = |outcome: Option<RunOutcome>| RunRecord {
        id: OperationId::new(),
        tool_id: ToolId::new("trivy").unwrap(),
        tool_name: "Trivy".into(),
        profile_id: "scan-folder".into(),
        profile_label: "Scan".into(),
        tool_version: None,
        started_at: "2026-01-01T00:00:00Z".into(),
        finished_at: outcome.map(|_| "2026-01-01T00:00:05Z".into()),
        duration_ms: None,
        exit_code: None,
        outcome,
        command: "trivy.exe fs".into(),
        targets: vec![],
        scope: "no network targets".into(),
        network_activity: NetworkActivity::None,
        output_format: OutputFormat::Text,
        run_dir: None,
        result_path: None,
    };
    let cut_short = record(None);
    let done = record(Some(RunOutcome::Findings));
    store.put_run(&cut_short).unwrap();
    store.put_run(&done).unwrap();
    assert_eq!(support.close_unfinished_runs(), 1);
    let fixed = store.get_run(cut_short.id).unwrap();
    assert_eq!(fixed.outcome, Some(RunOutcome::Failed));
    assert!(fixed.finished_at.is_some());
    assert_eq!(store.get_run(done.id).unwrap().outcome, Some(RunOutcome::Findings), "finished runs are not touched");
    assert_eq!(support.close_unfinished_runs(), 0, "idempotent");
}

#[cfg(windows)]
#[tokio::test]
async fn tab_separated_output_keeps_its_fields_apart() {
    use crate::adapters::executor::SystemCommandExecutor;
    use crate::domain::process::{CancellationToken, OutputStream};
    use crate::ports::{CommandExecutor, CommandSpec, LineSink};
    use std::sync::Mutex;
    #[derive(Default)]
    struct Collect(Mutex<Vec<String>>);
    impl LineSink for Collect {
        fn line(&self, _: OutputStream, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
    }
    let cmd = std::env::var_os("ComSpec").map(std::path::PathBuf::from).unwrap();
    let spec = CommandSpec { program: cmd, args: vec!["/C".into(), "echo 2026-10-19T19:21:16+05:30\tINFO\t[vulndb] Need to update DB".into()], timeout: std::time::Duration::from_secs(20) };
    let sink = Collect::default();
    SystemCommandExecutor.run(spec, &sink, &CancellationToken::new()).await.unwrap();
    let lines = sink.0.lock().unwrap().join("\n");
    assert!(lines.contains("+05:30 INFO [vulndb]") || lines.contains("+05:30    INFO    [vulndb]"), "fields must stay separated: {lines}");
    assert!(!lines.contains("30INFO"), "{lines}");
}
