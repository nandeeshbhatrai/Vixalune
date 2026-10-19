//! Opt-in integration test of the REAL container path: pulls the pinned Semgrep image (a few hundred MB),
//! verifies it, scans the practice sample project through `ContainerToolRunner` and removes the image again.
//! Needs Docker Desktop (Linux engine) running. Run explicitly:
//!
//!     $env:VIXALUNE_INTEGRATION_CONTAINER = "1"
//!     cargo test --manifest-path src-tauri/Cargo.toml --test container_integration -- --ignored --nocapture

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use vixalune_lib::adapters::activity_sqlite::SqliteActivityLogger;
use vixalune_lib::adapters::container::ContainerAdapter;
use vixalune_lib::adapters::container_runner::ContainerToolRunner;
use vixalune_lib::adapters::executor::SystemCommandExecutor;
use vixalune_lib::adapters::locator::SystemExecutableLocator;
use vixalune_lib::adapters::practice_lab::PracticeLabs;
use vixalune_lib::adapters::registry::ManifestRegistry;
use vixalune_lib::adapters::run_support::RunSupport;
use vixalune_lib::adapters::settings_store::FileSettings;
use vixalune_lib::adapters::sqlite_store::SqliteJobStore;
use vixalune_lib::adapters::system_net::{HttpRedirectProbe, SystemNameResolver};
use vixalune_lib::domain::container::{RunApprovals, DEFAULT_APPROVED_REGISTRIES};
use vixalune_lib::domain::jobs::JobPhase;
use vixalune_lib::domain::process::{CancellationToken, OutputStream};
use vixalune_lib::domain::runs::RunOutcome;
use vixalune_lib::domain::ToolId;
use vixalune_lib::ports::{AdapterContext, InstallerAdapter, JobStore, LabInfo, LineSink, RunHistory, ToolRegistry, ToolRunner, TrustLevel};

struct Print;
impl LineSink for Print {
    fn line(&self, _: OutputStream, t: &str) {
        println!("    | {t}");
    }
}

#[tokio::test]
#[ignore]
async fn real_semgrep_install_scan_and_cleanup() {
    if std::env::var("VIXALUNE_INTEGRATION_CONTAINER").is_err() {
        eprintln!("set VIXALUNE_INTEGRATION_CONTAINER=1 to run (pulls an image)");
        return;
    }
    let manifests = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests");
    let registry = Arc::new(ManifestRegistry::load_dir(&manifests, TrustLevel::Bundled).unwrap());
    let id = ToolId::new("semgrep").unwrap();
    let manifest = registry.get(&id).unwrap().manifest;
    let spec = manifest.install.iter().find(|s| matches!(s, vixalune_lib::domain::manifest::InstallSpec::ContainerImage { .. })).unwrap().clone();

    let exec = Arc::new(SystemCommandExecutor);
    let loc = Arc::new(SystemExecutableLocator);
    let regs: Vec<String> = DEFAULT_APPROVED_REGISTRIES.iter().map(|s| s.to_string()).collect();
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let adapter = Arc::new(ContainerAdapter::new(exec.clone(), loc.clone(), regs.clone(), home.clone()));
    let cancel = CancellationToken::new();
    let ctx = AdapterContext { tool: &manifest, cancel: &cancel, sink: &Print };

    println!("== install");
    let out = adapter.install(&ctx, &spec, false).await.expect("install");
    println!("{}", out.detail);
    println!("== verify");
    let v = adapter.verify(&ctx, &spec).await.expect("verify");
    println!("{:?} {:?} {}", v.outcome, v.level, v.detail);

    // Run support with a sample project under the user's profile (mounts refuse AppData).
    let dir = tempfile::tempdir().unwrap();
    let labs_dir = home.clone().unwrap().join("Vixalune").join("labs-integration-test");
    let labs = Arc::new(PracticeLabs::new(&labs_dir, 0));
    labs.ensure_sample_project().unwrap();
    let store = Arc::new(SqliteJobStore::in_memory().unwrap());
    let support = Arc::new(RunSupport {
        settings: Arc::new(FileSettings::in_memory()),
        labs: labs.clone(),
        history: store.clone(),
        resolver: Arc::new(SystemNameResolver),
        probe: Arc::new(HttpRedirectProbe),
        runs_dir: dir.path().join("runs"),
    });
    let runner = ContainerToolRunner::new(registry.clone(), exec.clone(), loc.clone(), adapter.clone(), store.clone(), Arc::new(SqliteActivityLogger::in_memory().unwrap()), regs, home).with_support(support);

    println!("== scan the sample project");
    let folder = labs.placeholders()["{lab:sample_folder}"].clone();
    let approvals = RunApprovals { mounts: HashMap::from([("source".to_string(), folder)]), ..Default::default() };
    let preview = runner.preview(&id, "scan-sample", &HashMap::new(), &approvals).await.expect("preview");
    println!("{}", preview.command.display);
    let op = runner.start(&id, "scan-sample", &HashMap::new(), &approvals).await.expect("start");
    let job = runner.wait(op, Duration::from_secs(300)).await.expect("wait");
    for l in store.job_output(op, 200).unwrap() {
        println!("    | {l}");
    }
    let rec = store.get_run(op).unwrap();
    println!("job={:?} exit={:?} outcome={:?}", job.phase, job.exit_code, rec.outcome);

    println!("== cleanup");
    let un = adapter.uninstall(&ctx, &spec).await;
    println!("{un:?}");
    let _ = std::fs::remove_dir_all(&labs_dir);

    assert_eq!(job.phase, JobPhase::Succeeded, "a scan with findings must be a successful run");
    assert_eq!(rec.outcome, Some(RunOutcome::Findings));
}
