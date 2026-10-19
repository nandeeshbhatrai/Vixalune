//! Opt-in integration tests against the REAL Windows Package Manager. They are `#[ignore]`d, so
//! `cargo test` never touches the machine. Run them explicitly:
//!
//! Read-only (no installation; needs network):
//!     cargo test --manifest-path src-tauri/Cargo.toml --test winget_integration -- --ignored read_only
//!
//! Real install + verify + uninstall of Trivy (changes this PC; needs network):
//!     $env:VIXALUNE_INTEGRATION_INSTALL = "1"
//!     cargo test --manifest-path src-tauri/Cargo.toml --test winget_integration -- --ignored --nocapture real_install
//!
//! Trivy is used because it is a small portable package that needs no administrator rights. The
//! install test refuses to run if Trivy is already installed, so it never removes a tool you use.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use vixalune_lib::adapters::activity_sqlite::SqliteActivityLogger;
use vixalune_lib::adapters::host_facts::WindowsHostFacts;
use vixalune_lib::adapters::platform_detector::SystemPlatformDetector;
use vixalune_lib::adapters::executor::SystemCommandExecutor;
use vixalune_lib::adapters::locator::SystemExecutableLocator;
use vixalune_lib::adapters::registry::ManifestRegistry;
use vixalune_lib::adapters::resolver::BasicDependencyResolver;
use vixalune_lib::adapters::sqlite_store::SqliteJobStore;
use vixalune_lib::adapters::winget::WingetAdapter;
use vixalune_lib::domain::install_state::InstallState;
use vixalune_lib::domain::jobs::{JobPhase, VerificationLevel, VerificationOutcome};
use vixalune_lib::domain::ToolId;
use vixalune_lib::engine::{EngineDeps, InstallEngine};
use vixalune_lib::ports::{InstallationManager, JobRegistry, JobStore, TrustLevel};

fn engine() -> (InstallEngine, Arc<SqliteJobStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let manifests = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests");
    let registry = ManifestRegistry::load_dir(&manifests, TrustLevel::Bundled).unwrap();
    assert!(registry.rejected.is_empty(), "{:?}", registry.rejected);
    let store = Arc::new(SqliteJobStore::open(&dir.path().join("it.db")).unwrap());
    let adapter = Arc::new(WingetAdapter::new(Arc::new(SystemCommandExecutor), Arc::new(SystemExecutableLocator)));
    let engine = InstallEngine::new(EngineDeps {
        registry: Arc::new(registry),
        detector: Arc::new(SystemPlatformDetector::new(
            Arc::new(SystemCommandExecutor),
            Arc::new(SystemExecutableLocator),
            Arc::new(WindowsHostFacts),
            dir.path().to_path_buf(),
            None,
        )),
        resolver: Arc::new(BasicDependencyResolver),
        adapters: vec![adapter],
        store: store.clone(),
        activity: Arc::new(SqliteActivityLogger::in_memory().unwrap()),
    });
    (engine, store, dir)
}

#[tokio::test]
#[ignore = "talks to the real winget; run with --ignored"]
async fn read_only_plan_and_detection_with_real_winget() {
    let (engine, _store, _dir) = engine();
    let trivy = ToolId::new("trivy").unwrap();
    let plan = engine.plan(&trivy).await.unwrap();
    println!("plan: {plan:#?}");
    assert!(plan.is_ready(), "winget must be installed for this test: {plan:?}");

    engine.refresh_states().await.unwrap();
    let states = engine.states().unwrap();
    for s in &states {
        println!("{} -> {:?}", s.tool_id, s.state);
    }
    // Container-only Semgrep has no adapter yet, so its state is never invented.
    let semgrep = states.iter().find(|s| s.tool_id.as_str() == "semgrep").unwrap();
    assert_eq!(semgrep.state, None);
    let trivy_state = states.iter().find(|s| s.tool_id == trivy).unwrap();
    assert!(matches!(trivy_state.state, Some(InstallState::NotInstalled | InstallState::Installed)));
}

#[tokio::test]
#[ignore = "installs software on this PC; needs VIXALUNE_INTEGRATION_INSTALL=1 and --ignored"]
async fn real_install_verify_and_uninstall_of_trivy() {
    if std::env::var("VIXALUNE_INTEGRATION_INSTALL").as_deref() != Ok("1") {
        eprintln!("skipped: set VIXALUNE_INTEGRATION_INSTALL=1 to run the real installation test");
        return;
    }
    let (engine, store, _dir) = engine();
    let trivy = ToolId::new("trivy").unwrap();
    engine.refresh_states().await.unwrap();
    if engine.state(&trivy).ok() == Some(InstallState::Installed) {
        eprintln!("skipped: Trivy is already installed on this PC; this test will not remove it");
        return;
    }

    let op = engine.install(&trivy, false).await.unwrap();
    let job = engine.wait_for_terminal(op, Duration::from_secs(15 * 60)).await.unwrap();
    let output = engine.output(op).unwrap();
    println!("--- install output ---\n{}\n--- end ---", output.join("\n"));
    println!("job: {job:#?}");

    // Always clean up what we installed, even if an assertion below fails.
    let cleanup = async {
        let op = engine.uninstall(&trivy, false).await.unwrap();
        engine.wait_for_terminal(op, Duration::from_secs(10 * 60)).await.unwrap()
    };

    if job.phase != JobPhase::Succeeded {
        let _ = cleanup.await;
        panic!("install did not succeed: {:?}", job.error);
    }
    let rec = store.get_installation(&trivy).unwrap().expect("installation record");
    println!("record: {rec:#?}");
    let v = rec.last_verification.clone().expect("verification result");
    let checks = (
        rec.state == InstallState::Installed,
        v.outcome == VerificationOutcome::Passed,
        v.level == VerificationLevel::Executable,
        rec.version.is_some(),
    );

    let uninstall = cleanup.await;
    println!("uninstall job: {uninstall:#?}");
    assert_eq!(checks, (true, true, true, true), "record/verification were not as expected: {rec:?}");
    assert_eq!(uninstall.phase, JobPhase::Succeeded, "{:?}", uninstall.error);
    assert!(store.get_installation(&trivy).unwrap().is_none());
    engine.refresh_states().await.unwrap();
    assert_eq!(engine.state(&trivy).unwrap(), InstallState::NotInstalled);
    assert_eq!(engine.list().unwrap().len(), 2);
}

