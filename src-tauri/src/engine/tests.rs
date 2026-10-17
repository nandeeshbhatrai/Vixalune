//! Engine tests. The real `WingetAdapter` and `SqliteJobStore` run against a simulated winget
//! (`MockExecutor`), so arguments, state transitions and persistence are all exercised for real.

use super::*;
use crate::adapters::activity_sqlite::SqliteActivityLogger;
use crate::adapters::registry::ManifestRegistry;
use crate::adapters::resolver::BasicDependencyResolver;
use crate::adapters::sqlite_store::SqliteJobStore;
use crate::adapters::testing::{MockExecutor, MockLocator, Step};
use crate::adapters::winget::{WingetAdapter, WINGET_NO_INSTALLED_PACKAGE};
use crate::domain::environment::{ComponentState, ComponentStatus, EnvironmentReport};
use crate::domain::manifest::fixtures::VALID;
use crate::domain::manifest::Prerequisite;
use crate::ports::{CommandSpec, TrustLevel};
use std::path::Path;
use std::sync::atomic::AtomicUsize;

const WAIT: Duration = Duration::from_secs(10);

/// What the simulated winget/tool currently does.
struct Sim {
    installed: AtomicBool,
    install_step: Mutex<Step>,
    /// The "installer" leaves files behind before it is cancelled or fails.
    partial_on_install: AtomicBool,
    exe_output: Mutex<String>,
    list_times_out_when_installed: AtomicBool,
    uninstall_is_noop: AtomicBool,
    extra_line: Mutex<Option<String>>,
    installs: AtomicUsize,
}

impl Sim {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            installed: AtomicBool::new(false),
            install_step: Mutex::new(Step::ok(&["Downloading", "  45%", "Successfully installed"])),
            partial_on_install: AtomicBool::new(false),
            exe_output: Mutex::new("Nmap version 7.80".into()),
            list_times_out_when_installed: AtomicBool::new(false),
            uninstall_is_noop: AtomicBool::new(false),
            extra_line: Mutex::new(None),
            installs: AtomicUsize::new(0),
        })
    }

    fn handler(self: &Arc<Self>) -> impl Fn(&CommandSpec) -> Step + Send + Sync + 'static {
        let sim = self.clone();
        move |c: &CommandSpec| {
            if c.program.ends_with("nmap.exe") {
                return Step::ok(&[sim.exe_output.lock().unwrap().as_str()]);
            }
            match c.args[0].as_str() {
                "show" => Step::ok(&["Found Nmap [Insecure.Nmap]", "Version: 7.99"]),
                "list" => {
                    if sim.installed.load(Ordering::SeqCst) {
                        if sim.list_times_out_when_installed.load(Ordering::SeqCst) {
                            Step::timed_out()
                        } else {
                            Step::ok(&["Name Id Version Source", "Nmap Insecure.Nmap 7.80 winget"])
                        }
                    } else {
                        Step::exit(WINGET_NO_INSTALLED_PACKAGE, &["No installed package found matching input criteria."])
                    }
                }
                "install" | "upgrade" => {
                    sim.installs.fetch_add(1, Ordering::SeqCst);
                    let step = sim.install_step.lock().unwrap().clone();
                    let succeeds = matches!(step.outcome, crate::ports::CommandOutcome::Exited(0)) && !step.block_until_cancel;
                    if succeeds || sim.partial_on_install.load(Ordering::SeqCst) {
                        sim.installed.store(true, Ordering::SeqCst);
                    }
                    let mut step = step;
                    if let Some(extra) = sim.extra_line.lock().unwrap().clone() {
                        step.lines.push(extra);
                    }
                    step
                }
                "uninstall" => {
                    if !sim.uninstall_is_noop.load(Ordering::SeqCst) {
                        sim.installed.store(false, Ordering::SeqCst);
                    }
                    Step::ok(&["Successfully uninstalled"])
                }
                other => panic!("unexpected winget subcommand {other}"),
            }
        }
    }
}

fn env(winget: bool) -> EnvironmentReport {
    EnvironmentReport {
        os: "windows".into(),
        supported_os: true,
        windows_build: Some(26100),
        architecture: "x64".into(),
        components: vec![ComponentStatus {
            component: Prerequisite::Winget,
            state: if winget { ComponentState::Available { version: Some("v1".into()) } } else { ComponentState::Missing },
        }],
        issues: vec![],
    }
}

struct FakeDetector(EnvironmentReport);
impl EnvironmentDetector for FakeDetector {
    fn detect(&self) -> EnvironmentReport {
        self.0.clone()
    }
}

struct Harness {
    engine: InstallEngine,
    store: Arc<SqliteJobStore>,
    sim: Arc<Sim>,
    exec: Arc<MockExecutor>,
    activity: Arc<SqliteActivityLogger>,
    _dir: tempfile::TempDir,
}

fn nmap_id() -> ToolId {
    ToolId::new("nmap").unwrap()
}

fn write_manifests(dir: &Path, manifests: &[(&str, String)]) {
    for (file, body) in manifests {
        std::fs::write(dir.join(file), body).unwrap();
    }
}

fn build(env: EnvironmentReport, manifests: &[(&str, String)], store: Arc<SqliteJobStore>, dir: tempfile::TempDir) -> Harness {
    let sim = Sim::new();
    let exec = Arc::new(MockExecutor::new(sim.handler()));
    let mut loc = MockLocator::default();
    loc.add("winget.exe", "C:\\fake\\winget.exe");
    loc.add("nmap.exe", "C:\\Program Files (x86)\\Nmap\\nmap.exe");
    let adapter: Arc<dyn InstallerAdapter> = Arc::new(WingetAdapter::new(exec.clone(), Arc::new(loc)));
    let mdir = dir.path().join("manifests");
    std::fs::create_dir_all(&mdir).unwrap();
    write_manifests(&mdir, manifests);
    let registry = ManifestRegistry::load_dir(&mdir, TrustLevel::Bundled).unwrap();
    assert!(registry.rejected.is_empty(), "{:?}", registry.rejected);
    let activity = Arc::new(SqliteActivityLogger::in_memory().unwrap());
    let engine = InstallEngine::new(EngineDeps {
        registry: Arc::new(registry),
        detector: Arc::new(FakeDetector(env)),
        resolver: Arc::new(BasicDependencyResolver),
        adapters: vec![adapter],
        store: store.clone(),
        activity: activity.clone(),
    });
    Harness { engine, store, sim, exec, activity, _dir: dir }
}

fn harness_with(env: EnvironmentReport, manifests: &[(&str, String)]) -> Harness {
    build(env, manifests, Arc::new(SqliteJobStore::in_memory().unwrap()), tempfile::tempdir().unwrap())
}

fn harness() -> Harness {
    harness_with(env(true), &[("nmap.toml", VALID.to_string())])
}

impl Harness {
    fn commands(&self, sub: &str) -> usize {
        self.exec.calls().iter().filter(|c| c.program.ends_with("winget.exe") && c.args[0] == sub).count()
    }
    async fn cancel(&self, op: OperationId) -> AppResult<()> {
        InstallationManager::cancel(&self.engine, op).await
    }
    async fn finish(&self, op: OperationId) -> Job {
        self.engine.wait_for_terminal(op, WAIT).await.unwrap()
    }
    async fn install_ok(&self) -> Job {
        let op = self.engine.install(&nmap_id(), true).await.unwrap();
        let job = self.finish(op).await;
        assert_eq!(job.phase, JobPhase::Succeeded, "{job:?}");
        job
    }
}

// ------------------------------------------------------------------ happy path

#[tokio::test]
async fn successful_install_is_verified_persisted_and_audited() {
    let h = harness();
    let job = h.install_ok().await;
    assert_eq!(job.exit_code, Some(0));
    assert!(job.error.is_none());
    assert!(job.finished_at.is_some());

    // Exact, fixed command lines were used.
    let install = h.exec.calls().into_iter().find(|c| c.args[0] == "install").unwrap();
    assert_eq!(install.program, Path::new("C:\\fake\\winget.exe"));
    assert_eq!(install.args[..4], ["install", "--id", "Insecure.Nmap", "--exact"]);

    // Verification (record + executable) ran after the install.
    assert!(h.exec.calls().iter().any(|c| c.program.ends_with("nmap.exe") && c.args == ["--version"]));

    let rec = h.store.get_installation(&nmap_id()).unwrap().unwrap();
    assert_eq!(rec.state, InstallState::Installed);
    assert_eq!(rec.version.as_deref(), Some("7.80"));
    assert_eq!(rec.location.as_deref(), Some("C:\\Program Files (x86)\\Nmap"));
    let v = rec.last_verification.unwrap();
    assert_eq!((v.outcome, v.level), (VerificationOutcome::Passed, VerificationLevel::Executable));

    let out = h.engine.output(job.id).unwrap();
    assert!(out.iter().any(|l| l.starts_with("> winget install")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("Successfully installed")));

    let states = h.engine.states().unwrap();
    assert_eq!(states[0].state, Some(InstallState::Installed));
    assert_eq!(h.engine.installed().unwrap()[0].tool_id, nmap_id());
    let events = h.activity.recent(20).unwrap();
    assert!(events.iter().any(|e| e.kind == ActivityKind::InstallSucceeded));
    assert!(events.iter().any(|e| e.kind == ActivityKind::InstallStarted));
}

#[tokio::test]
async fn progress_percent_is_parsed_from_output() {
    assert_eq!(parse_percent("  â–ˆâ–ˆâ–ˆâ–ˆâ–ˆâ–ˆâ–’â–’â–’â–’  45%"), Some(45));
    assert_eq!(parse_percent("100%"), Some(100));
    assert_eq!(parse_percent("1024 KB / 3.00 MB"), None);
    assert_eq!(parse_percent("150%"), None);
    let h = harness();
    let id = h.install_ok().await.id;
    // The simulated installer printed 45%; it is recorded on the job.
    assert_eq!(h.store.get_job(id).unwrap().percent, Some(45));
}

// ------------------------------------------------------------------ verification gates success

#[tokio::test]
async fn failed_verification_is_never_reported_as_installed() {
    let h = harness();
    *h.sim.exe_output.lock().unwrap() = "garbage".into();
    let op = h.engine.install(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Failed);
    let err = job.error.unwrap();
    assert_eq!(err.code, "verification_failed");
    assert!(err.detail.unwrap().contains("did not contain"));
    let rec = h.store.get_installation(&nmap_id()).unwrap().unwrap();
    assert_eq!(rec.state, InstallState::VerificationFailed);
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::VerificationFailed));
    // The tool is present (not rolled back): the user decides what to do.
    assert!(h.sim.installed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn unavailable_verification_is_distinct_from_success() {
    let h = harness();
    h.sim.list_times_out_when_installed.store(true, Ordering::SeqCst);
    let op = h.engine.install(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Failed);
    assert_eq!(job.error.unwrap().code, "verification_unavailable");
}

#[tokio::test]
async fn verify_job_reports_current_state() {
    let h = harness();
    h.install_ok().await;
    *h.sim.exe_output.lock().unwrap() = "broken".into();
    let op = h.engine.verify(&nmap_id()).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Failed);
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::VerificationFailed));
    *h.sim.exe_output.lock().unwrap() = "Nmap version 7.80".into();
    let op = h.engine.verify(&nmap_id()).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Succeeded);
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::Installed));
}

// ------------------------------------------------------------------ failures and retry

#[tokio::test]
async fn failed_install_reports_exit_code_output_and_recovery_then_retry_succeeds() {
    let h = harness();
    *h.sim.install_step.lock().unwrap() = Step::exit(0x8A15_0010_u32 as i32, &["Installer failed", "Error 1603"]);
    let op = h.engine.install(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Failed);
    assert_eq!(job.exit_code, Some(0x8A15_0010_u32 as i32));
    let err = job.error.clone().unwrap();
    assert_eq!(err.code, "command_failed");
    assert!(err.message.contains("0x8A150010"), "{}", err.message);
    assert!(err.detail.as_deref().unwrap().contains("Error 1603"));
    assert!(!err.recovery.is_empty());
    assert!(h.store.get_installation(&nmap_id()).unwrap().is_none());
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::Failed));
    assert!(h.engine.output(job.id).unwrap().iter().any(|l| l.contains("Error 1603")));

    // Retry after fixing the cause.
    *h.sim.install_step.lock().unwrap() = Step::ok(&["Successfully installed"]);
    let retry = h.engine.retry(&nmap_id(), true).await.unwrap();
    assert_ne!(retry, op);
    assert_eq!(h.finish(retry).await.phase, JobPhase::Succeeded);
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::Installed));
    // Nothing left to retry now.
    assert!(matches!(h.engine.retry(&nmap_id(), true).await, Err(AppError::Validation(_))));
}

#[tokio::test]
async fn install_timeout_is_a_distinct_failure() {
    let h = harness();
    *h.sim.install_step.lock().unwrap() = Step::timed_out();
    let op = h.engine.install(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.error.unwrap().code, "timed_out");
}

#[tokio::test]
async fn missing_prerequisite_fails_before_touching_the_system() {
    let h = harness_with(env(false), &[("nmap.toml", VALID.to_string())]);
    let op = h.engine.install(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Failed);
    let err = job.error.unwrap();
    assert_eq!(err.code, "prerequisite_missing");
    assert!(err.message.contains("Winget") && err.message.contains("Windows Package Manager"), "{}", err.message);
    assert!(h.exec.calls().is_empty(), "no command may run when a prerequisite is missing");
}

#[tokio::test]
async fn unmet_tool_dependency_blocks_install() {
    let dependent = VALID.replace("id = \"nmap\"", "id = \"zenmap\"").replace("platforms = [\"windows_native\"]", "platforms = [\"windows_native\"]\ndependencies = [\"nmap\"]");
    let h = harness_with(env(true), &[("nmap.toml", VALID.to_string()), ("zenmap.toml", dependent)]);
    let zen = ToolId::new("zenmap").unwrap();
    let job = h.finish(h.engine.install(&zen, true).await.unwrap()).await;
    assert_eq!(job.error.unwrap().code, "prerequisite_missing");
    h.install_ok().await;
    assert_eq!(h.finish(h.engine.install(&zen, true).await.unwrap()).await.phase, JobPhase::Succeeded);
}

#[tokio::test]
async fn unsupported_adapter_is_rejected_without_creating_a_job() {
    let semgrep = include_str!("../../../manifests/semgrep.toml").to_string();
    let h = harness_with(env(true), &[("semgrep.toml", semgrep)]);
    let id = ToolId::new("semgrep").unwrap();
    let err = h.engine.install(&id, true).await.unwrap_err();
    assert!(matches!(err, AppError::Unsupported(_)), "{err:?}");
    assert!(err.to_string().contains("ContainerImage"));
    assert!(h.engine.list().unwrap().is_empty());
    assert!(h.exec.calls().is_empty());
    // The plan explains why instead of pretending.
    assert!(h.engine.plan(&id).await.unwrap().unsupported_reason.is_some());
    assert_eq!(h.engine.supported_methods(), vec![InstallMethod::Winget]);
}

#[tokio::test]
async fn unknown_tool_and_lifecycle_limits() {
    let h = harness();
    assert!(matches!(h.engine.install(&ToolId::new("nope").unwrap(), true).await, Err(AppError::NotFound(_))));
    let no_update = VALID.replace("can_update = true", "can_update = false").replace("can_uninstall = true", "can_uninstall = false");
    let h = harness_with(env(true), &[("nmap.toml", no_update)]);
    assert!(matches!(h.engine.update(&nmap_id(), true).await, Err(AppError::Unsupported(_))));
    assert!(matches!(h.engine.uninstall(&nmap_id(), true).await, Err(AppError::Unsupported(_))));
}

#[tokio::test]
async fn output_secrets_are_redacted_before_storage() {
    let h = harness();
    *h.sim.extra_line.lock().unwrap() = Some("proxy --password hunter2 token=abc123".into());
    let job = h.install_ok().await;
    let out = h.engine.output(job.id).unwrap().join("\n");
    assert!(!out.contains("hunter2") && !out.contains("abc123"), "{out}");
    assert!(out.contains("[REDACTED]"));
}

// ------------------------------------------------------------------ consent

#[tokio::test]
async fn elevation_requires_consent_and_waits_without_running_the_installer() {
    let h = harness();
    let op = h.engine.install(&nmap_id(), false).await.unwrap();
    let job = h.engine.wait_for_phase(op, JobPhase::AwaitingConsent, WAIT).await.unwrap();
    assert!(job.message.contains("approval"));
    assert_eq!(h.sim.installs.load(Ordering::SeqCst), 0);
    h.engine.approve(op).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Succeeded);
    assert_eq!(h.sim.installs.load(Ordering::SeqCst), 1);
    // Approving a finished job is an error.
    assert!(matches!(h.engine.approve(op).await, Err(AppError::InvalidTransition { .. })));
}

#[tokio::test]
async fn declining_consent_by_cancelling_never_runs_the_installer() {
    let h = harness();
    let op = h.engine.install(&nmap_id(), false).await.unwrap();
    h.engine.wait_for_phase(op, JobPhase::AwaitingConsent, WAIT).await.unwrap();
    h.cancel(op).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Cancelled);
    assert_eq!(h.sim.installs.load(Ordering::SeqCst), 0);
    assert_eq!(h.commands("install"), 0);
}

#[tokio::test]
async fn no_consent_needed_when_the_tool_needs_no_elevation() {
    let no_admin = VALID.replace("install = \"possible\"", "install = \"none\"");
    let h = harness_with(env(true), &[("nmap.toml", no_admin)]);
    let op = h.engine.install(&nmap_id(), false).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Succeeded);
}

// ------------------------------------------------------------------ cancellation and rollback

#[tokio::test]
async fn cancelling_an_install_rolls_back_a_partial_installation() {
    let h = harness();
    *h.sim.install_step.lock().unwrap() = Step::hang(&["Downloading"]);
    h.sim.partial_on_install.store(true, Ordering::SeqCst);
    let op = h.engine.install(&nmap_id(), true).await.unwrap();
    h.engine.wait_for_phase(op, JobPhase::Installing, WAIT).await.unwrap();
    h.cancel(op).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Cancelled);
    assert!(!h.sim.installed.load(Ordering::SeqCst), "partial install must be removed");
    assert_eq!(h.commands("uninstall"), 1);
    assert!(h.engine.output(op).unwrap().iter().any(|l| l.contains("Rolling back")));
    assert!(h.store.get_installation(&nmap_id()).unwrap().is_none());
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::NotInstalled));
}

#[tokio::test]
async fn cancel_does_not_remove_a_tool_the_user_already_had() {
    let h = harness();
    h.sim.installed.store(true, Ordering::SeqCst);
    // An update is cancelled mid-way: the pre-existing installation must stay.
    *h.sim.install_step.lock().unwrap() = Step::hang(&["Upgrading"]);
    let op = h.engine.update(&nmap_id(), true).await.unwrap();
    h.engine.wait_for_phase(op, JobPhase::Installing, WAIT).await.unwrap();
    h.cancel(op).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Cancelled);
    assert_eq!(h.commands("uninstall"), 0);
    assert!(h.sim.installed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancelling_a_finished_or_unknown_job_is_an_error() {
    let h = harness();
    let id = h.install_ok().await.id;
    assert!(matches!(h.cancel(id).await, Err(AppError::InvalidTransition { .. })));
    assert!(matches!(h.cancel(OperationId::new()).await, Err(AppError::NotFound(_))));
}

// ------------------------------------------------------------------ concurrency

#[tokio::test]
async fn duplicate_requests_are_rejected_while_an_operation_is_active() {
    let h = harness();
    *h.sim.install_step.lock().unwrap() = Step::hang(&["Downloading"]);
    let first = h.engine.install(&nmap_id(), true).await.unwrap();
    h.engine.wait_for_phase(first, JobPhase::Installing, WAIT).await.unwrap();
    for result in [
        h.engine.install(&nmap_id(), true).await,
        h.engine.uninstall(&nmap_id(), true).await,
        h.engine.verify(&nmap_id()).await,
        h.engine.update(&nmap_id(), true).await,
        h.engine.retry(&nmap_id(), true).await,
    ] {
        assert!(matches!(result, Err(AppError::Busy(_))), "{result:?}");
    }
    assert_eq!(h.engine.list().unwrap().len(), 1, "rejected requests must not create jobs");
    assert_eq!(h.sim.installs.load(Ordering::SeqCst), 1);
    h.cancel(first).await.unwrap();
    h.finish(first).await;
    // The tool is free again.
    *h.sim.install_step.lock().unwrap() = Step::ok(&["ok"]);
    let again = h.engine.install(&nmap_id(), true).await.unwrap();
    assert_eq!(h.finish(again).await.phase, JobPhase::Succeeded);
}

#[tokio::test]
async fn simultaneous_requests_start_exactly_one_installation() {
    let h = harness();
    *h.sim.install_step.lock().unwrap() = Step::ok(&["ok"]).delayed(100);
    let id = nmap_id();
    let (a, b, c) = tokio::join!(h.engine.install(&id, true), h.engine.install(&id, true), h.engine.install(&id, true));
    let results = [a, b, c];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
    assert_eq!(results.iter().filter(|r| matches!(r, Err(AppError::Busy(_)))).count(), 2);
    let ok = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(h.finish(ok).await.phase, JobPhase::Succeeded);
    assert_eq!(h.sim.installs.load(Ordering::SeqCst), 1);
}

// ------------------------------------------------------------------ existing installs, uninstall, update

#[tokio::test]
async fn existing_installation_is_verified_not_reinstalled() {
    let h = harness();
    h.sim.installed.store(true, Ordering::SeqCst);
    let job = h.install_ok().await;
    assert_eq!(h.sim.installs.load(Ordering::SeqCst), 0);
    assert!(job.message.contains("verified"));
}

#[tokio::test]
async fn uninstall_removes_the_record_and_confirms_with_the_package_manager() {
    let h = harness();
    h.install_ok().await;
    let op = h.engine.uninstall(&nmap_id(), true).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Succeeded);
    assert!(h.store.get_installation(&nmap_id()).unwrap().is_none());
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::NotInstalled));
    assert_eq!(h.commands("uninstall"), 1);
    // Uninstalling something that is not installed is a harmless no-op, not an error.
    let op = h.engine.uninstall(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Succeeded);
    assert_eq!(h.commands("uninstall"), 1);
}

#[tokio::test]
async fn uninstall_that_did_not_remove_the_package_is_not_reported_as_success() {
    let h = harness();
    h.install_ok().await;
    h.sim.uninstall_is_noop.store(true, Ordering::SeqCst);
    let op = h.engine.uninstall(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Failed);
    assert_eq!(job.error.unwrap().code, "verification_failed");
    assert!(h.store.get_installation(&nmap_id()).unwrap().is_some());
}

#[tokio::test]
async fn update_uses_upgrade_and_requires_an_existing_install() {
    let h = harness();
    let op = h.engine.update(&nmap_id(), true).await.unwrap();
    let job = h.finish(op).await;
    assert_eq!(job.phase, JobPhase::Failed);
    assert_eq!(job.error.unwrap().code, "validation");
    h.install_ok().await;
    let op = h.engine.update(&nmap_id(), true).await.unwrap();
    assert_eq!(h.finish(op).await.phase, JobPhase::Succeeded);
    assert_eq!(h.commands("upgrade"), 1);
}

#[tokio::test]
async fn plan_explains_requirements_before_anything_runs() {
    let h = harness();
    let plan = h.engine.plan(&nmap_id()).await.unwrap();
    assert!(plan.is_ready());
    assert_eq!(plan.elevation, crate::domain::manifest::ElevationNeed::Possible);
    assert!(plan.network_required);
    assert!(plan.prerequisites[0].spec.reason.contains("Windows Package Manager"));
    assert!(h.exec.calls().is_empty());
}

// ------------------------------------------------------------------ detection

#[tokio::test]
async fn refresh_detects_real_state_and_unknown_stays_unknown() {
    let h = harness();
    assert_eq!(h.engine.states().unwrap()[0].state, None, "nothing is known before detection");
    h.engine.refresh_states().await.unwrap();
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::NotInstalled));
    h.sim.installed.store(true, Ordering::SeqCst);
    h.engine.refresh_states().await.unwrap();
    assert_eq!(h.engine.states().unwrap()[0].state, Some(InstallState::Installed));
    let rec = h.store.get_installation(&nmap_id()).unwrap().unwrap();
    assert!(rec.last_verification.is_none(), "a detected install has not been verified by Vixalune");
    assert_eq!(rec.version.as_deref(), Some("7.80"));
}

// ------------------------------------------------------------------ restart and recovery

#[tokio::test]
async fn state_survives_restart_and_interrupted_jobs_are_closed() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("vixalune.db");
    let crashed = Job {
        id: OperationId::new(),
        tool_id: nmap_id(),
        kind: JobKind::Install,
        phase: JobPhase::Installing,
        message: "Installing Nmap".into(),
        percent: Some(30),
        started_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:01Z".into(),
        finished_at: None,
        exit_code: None,
        allow_elevation: true,
        error: None,
    };
    {
        // "First run": an install was in flight and a different tool state was recorded; then the app died.
        let store = SqliteJobStore::open(&db).unwrap();
        store.create_job(&crashed).unwrap();
        store.append_output(crashed.id, OutputStream::Stdout, "Downloading").unwrap();
    }
    // "Second run".
    let store = Arc::new(SqliteJobStore::open(&db).unwrap());
    let h = build(env(true), &[("nmap.toml", VALID.to_string())], store, tempfile::tempdir().unwrap());
    // Before recovery the tool is considered busy.
    assert!(matches!(h.engine.install(&nmap_id(), true).await, Err(AppError::Busy(_))));
    let closed = h.engine.recover().unwrap();
    assert_eq!(closed.len(), 1);
    let job = h.store.get_job(crashed.id).unwrap();
    assert_eq!(job.phase, JobPhase::Failed);
    let err = job.error.unwrap();
    assert_eq!(err.code, "interrupted");
    assert!(err.recovery.contains("Verify"));
    assert_eq!(h.engine.output(crashed.id).unwrap(), vec!["Downloading"]);
    // History is visible to the UI and the tool can be installed again.
    assert_eq!(h.engine.list().unwrap()[0].id, crashed.id);
    h.install_ok().await;

    // A third run still sees the installation record.
    drop(h);
    let store = SqliteJobStore::open(&db).unwrap();
    let rec = store.get_installation(&nmap_id()).unwrap().unwrap();
    assert_eq!(rec.state, InstallState::Installed);
    assert_eq!(store.list_jobs(10).unwrap().len(), 2);
}
