//! Setup manager: the few system changes Vixalune may perform, each only after explicit confirmation.
//!
//! * Only actions the platform report marks `Automatic` can run (installing WSL, installing a
//!   distribution). Everything else (BIOS, Docker Desktop, disk space) is guidance only.
//! * An action runs as a normal tracked job (progress, output, audit trail, recovery on restart).
//! * Elevation is one fixed command through Windows' own UAC prompt (see `elevation.rs`).
//! * A restart is **never** started implicitly: `request_restart` needs `confirmed == true`, uses a
//!   delay the user can cancel, and is a separate step from installing anything.
//! * After every action the environment is re-detected; "done" means the new state was observed.

use crate::domain::activity::{ActivityEvent, ActivityKind};
use crate::domain::failure::{classify, FailureKind, FailureLayer, OutputSource, PlatformFailure};
use crate::domain::jobs::{Job, JobError, JobEvent, JobKind, JobPhase};
use crate::domain::platform::{PlatformReport, SetupActionId, SetupKind};
use crate::domain::process::{CancellationToken, OutputStream};
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::{
    ActivityLogger, CommandExecutor, CommandOutcome, CommandSpec, ElevatedRunner, ExecutableLocator, JobStore, LineSink, PlatformDetector, SetupManager,
};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The only distribution Vixalune installs by itself (a name from Microsoft's `wsl --list --online`).
pub const DEFAULT_DISTRO: &str = "Ubuntu-24.04";
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct SetupEngine {
    inner: Arc<Inner>,
}

struct Inner {
    detector: Arc<dyn PlatformDetector>,
    executor: Arc<dyn CommandExecutor>,
    elevated: Arc<dyn ElevatedRunner>,
    locator: Arc<dyn ExecutableLocator>,
    store: Arc<dyn JobStore>,
    activity: Arc<dyn ActivityLogger>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn job_tool(id: SetupActionId) -> ToolId {
    let name = match id {
        SetupActionId::InstallWsl => "setup-install-wsl",
        SetupActionId::InstallDistro => "setup-install-distro",
        _ => "setup-manual",
    };
    ToolId::new(name).expect("static slug")
}

struct SinkToJob {
    store: Arc<dyn JobStore>,
    id: OperationId,
}
impl LineSink for SinkToJob {
    fn line(&self, stream: OutputStream, text: &str) {
        let _ = self.store.append_output(self.id, stream, text);
    }
}

impl SetupEngine {
    pub fn new(
        detector: Arc<dyn PlatformDetector>,
        executor: Arc<dyn CommandExecutor>,
        elevated: Arc<dyn ElevatedRunner>,
        locator: Arc<dyn ExecutableLocator>,
        store: Arc<dyn JobStore>,
        activity: Arc<dyn ActivityLogger>,
    ) -> Self {
        Self { inner: Arc::new(Inner { detector, executor, elevated, locator, store, activity }) }
    }

    pub async fn wait(&self, op: OperationId, timeout: Duration) -> AppResult<Job> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let j = self.inner.store.get_job(op)?;
            if j.phase.is_terminal() {
                return Ok(j);
            }
            if tokio::time::Instant::now() > deadline {
                return Err(AppError::TimedOut("setup job".into()));
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
}

impl Inner {
    fn audit(&self, kind: ActivityKind, tool: &ToolId, summary: &str) {
        let _ = self.activity.record(ActivityEvent { at: now(), kind, tool_id: Some(tool.clone()), summary: summary.into() });
    }

    fn system32(&self, exe: &str) -> AppResult<PathBuf> {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let p = PathBuf::from(root).join("System32").join(exe);
            if p.exists() {
                return Ok(p);
            }
        }
        self.locator.locate(exe, &[]).ok_or_else(|| AppError::PrerequisiteMissing(format!("{exe} was not found")))
    }

    fn step(&self, job: &mut Job, event: JobEvent, message: &str) -> AppResult<()> {
        job.phase = job.phase.apply(event)?;
        job.message = message.to_string();
        job.updated_at = now();
        if job.phase.is_terminal() {
            job.finished_at = Some(job.updated_at.clone());
        }
        self.store.update_job(job)
    }

    fn fail(&self, job: &mut Job, e: &AppError) {
        job.error = Some(JobError::from_app(e, None));
        if let AppError::CommandFailed { code, .. } = e {
            job.exit_code = *code;
        }
        if self.step(job, JobEvent::Fail, &e.to_string()).is_err() {
            job.phase = JobPhase::Failed;
            job.finished_at = Some(now());
            let _ = self.store.update_job(job);
        }
        self.audit(ActivityKind::SetupFailed, &job.tool_id, &format!("{} ({})", e.code(), job.tool_id));
    }

    async fn execute(&self, action: SetupActionId, job: &mut Job) -> AppResult<()> {
        // 1. Re-check: the action must still be needed and its prerequisites finished.
        self.step(job, JobEvent::StartChecks, "Checking the current state")?;
        let before: PlatformReport = self.detector.detect().await;
        let Some(proposed) = before.setup_actions.iter().find(|a| a.id == action) else {
            return Err(AppError::Validation("this step is not needed (or not possible) on this computer right now; re-check your environment".into()));
        };
        if proposed.kind != SetupKind::Automatic {
            return Err(AppError::Unsupported("this step must be done by you; Vixalune only explains it".into()));
        }
        if let Some(dep) = proposed.depends_on.iter().find(|d| before.setup_actions.iter().any(|a| a.id == **d)) {
            return Err(AppError::PrerequisiteMissing(format!("finish {dep:?} first (and restart Windows if asked)")));
        }

        // 2. Do exactly the previewed command.
        self.step(job, JobEvent::BeginInstall, &proposed.title)?;
        let wsl = self.system32("wsl.exe")?;
        match action {
            SetupActionId::InstallWsl => {
                self.store.append_output(job.id, OutputStream::Stdout, "> (administrator) wsl.exe --install --no-distribution")?;
                self.store.append_output(job.id, OutputStream::Stdout, "Windows will ask for your permission. Output of an elevated command cannot be shown here.")?;
                let out = self.elevated.run(&wsl, &["--install".to_string(), "--no-distribution".to_string()], SETUP_TIMEOUT).await?;
                if out.declined {
                    return Err(AppError::Platform(PlatformFailure::new(FailureKind::MissingPermissions, FailureLayer::Windows, "you declined the administrator prompt; nothing was changed")));
                }
                match out.exit_code {
                    Some(0) => {}
                    Some(code) => {
                        return Err(AppError::CommandFailed {
                            code: Some(code),
                            message: format!("wsl --install exited with 0x{:08X}. Troubleshooting guide: https://learn.microsoft.com/windows/wsl/troubleshooting", code as u32),
                        })
                    }
                    None => return Err(AppError::Io("no exit code was returned".into())),
                }
            }
            SetupActionId::InstallDistro => {
                let args: Vec<String> = ["--install", "--distribution", DEFAULT_DISTRO, "--no-launch"].map(String::from).to_vec();
                self.store.append_output(job.id, OutputStream::Stdout, &format!("> wsl.exe {}", args.join(" ")))?;
                let sink = SinkToJob { store: self.store.clone(), id: job.id };
                let r = self.executor.run(CommandSpec { program: wsl, args, timeout: SETUP_TIMEOUT }, &sink, &CancellationToken::new()).await?;
                match r.outcome {
                    CommandOutcome::Exited(0) => {}
                    CommandOutcome::Exited(code) => {
                        if let Some(f) = classify(OutputSource::Wsl, Some(code), &r.tail) {
                            return Err(AppError::Platform(f));
                        }
                        return Err(AppError::CommandFailed { code: Some(code), message: format!("wsl --install exited with {code}. {}", r.tail.last().cloned().unwrap_or_default()) });
                    }
                    CommandOutcome::TimedOut => return Err(AppError::TimedOut("installing the distribution".into())),
                    CommandOutcome::Cancelled => return Err(AppError::Cancelled),
                }
            }
            _ => return Err(AppError::Unsupported("not an automatic step".into())),
        }

        // 3. Observe the result. Success is what the re-check shows, not what the command said.
        self.step(job, JobEvent::InstallDone, "Re-checking your environment")?;
        let after = self.detector.detect().await;
        match action {
            SetupActionId::InstallWsl => {
                if after.wsl.installed == Some(true) {
                    self.step(job, JobEvent::VerifyPassed, "WSL is installed")?;
                } else if after.system.pending_restart == Some(true) {
                    self.step(job, JobEvent::VerifyPassed, "WSL was installed. Restart Windows to finish; Vixalune will not restart it without your confirmation.")?;
                } else {
                    let e = AppError::VerificationFailed("the command finished but WSL is not detected and no restart is pending".into());
                    job.error = Some(JobError::from_app(&e, None));
                    self.step(job, JobEvent::VerifyFailed, "WSL was not detected after installation")?;
                    self.audit(ActivityKind::SetupFailed, &job.tool_id, "WSL not detected after install");
                    return Ok(());
                }
            }
            SetupActionId::InstallDistro => {
                if after.wsl.usable_distros().any(|d| d.name == DEFAULT_DISTRO) {
                    self.step(job, JobEvent::VerifyPassed, &format!("{DEFAULT_DISTRO} is installed (WSL 2)"))?;
                } else {
                    let e = AppError::VerificationFailed(format!("{DEFAULT_DISTRO} is not listed as a WSL 2 distribution after installing"));
                    job.error = Some(JobError::from_app(&e, None));
                    self.step(job, JobEvent::VerifyFailed, "the distribution was not detected after installation")?;
                    self.audit(ActivityKind::SetupFailed, &job.tool_id, "distribution not detected after install");
                    return Ok(());
                }
            }
            _ => {}
        }
        self.audit(ActivityKind::SetupSucceeded, &job.tool_id, &job.message.clone());
        Ok(())
    }
}

#[async_trait]
impl SetupManager for SetupEngine {
    async fn run_action(&self, id: SetupActionId, confirmed: bool) -> AppResult<OperationId> {
        if !confirmed {
            return Err(AppError::Validation("this changes your computer, so it needs your explicit confirmation".into()));
        }
        if !matches!(id, SetupActionId::InstallWsl | SetupActionId::InstallDistro) {
            return Err(AppError::Unsupported("this step must be done by you; follow the instructions shown in Setup".into()));
        }
        let tool = job_tool(id);
        let ts = now();
        let mut job = Job {
            id: OperationId::new(),
            tool_id: tool.clone(),
            kind: JobKind::Setup,
            phase: JobPhase::Queued,
            message: "Queued".into(),
            percent: None,
            started_at: ts.clone(),
            updated_at: ts,
            finished_at: None,
            exit_code: None,
            allow_elevation: true,
            error: None,
        };
        // One setup step at a time: they change shared system state.
        for other in [SetupActionId::InstallWsl, SetupActionId::InstallDistro] {
            if other != id && self.inner.store.active_job_for(&job_tool(other))?.is_some() {
                return Err(AppError::Busy("another setup step is already running".into()));
            }
        }
        self.inner.store.create_job(&job)?; // Busy if this very step is already running.
        self.inner.audit(ActivityKind::SetupStarted, &tool, &format!("{id:?} confirmed by the user"));
        let inner = self.inner.clone();
        let op = job.id;
        tokio::spawn(async move {
            if let Err(e) = inner.execute(id, &mut job).await {
                inner.fail(&mut job, &e);
            }
        });
        Ok(op)
    }

    async fn request_restart(&self, confirmed: bool, delay_secs: u32) -> AppResult<()> {
        if !confirmed {
            return Err(AppError::Validation("restarting Windows needs your explicit confirmation".into()));
        }
        let delay = delay_secs.clamp(30, 600);
        let exe = self.inner.system32("shutdown.exe")?;
        let args = vec!["/r".to_string(), "/t".into(), delay.to_string(), "/c".into(), "Vixalune: Windows will restart to finish setup. Save your work.".into()];
        let r = self.inner.executor.run(CommandSpec { program: exe, args, timeout: Duration::from_secs(30) }, &NullSink, &CancellationToken::new()).await?;
        match r.outcome {
            CommandOutcome::Exited(0) => {
                self.inner.audit(ActivityKind::RestartRequested, &ToolId::new("setup-restart").expect("slug"), &format!("restart scheduled in {delay}s after explicit confirmation"));
                Ok(())
            }
            CommandOutcome::Exited(code) => Err(AppError::CommandFailed { code: Some(code), message: format!("shutdown.exe exited with {code}. {}", r.tail.join(" | ")) }),
            CommandOutcome::TimedOut => Err(AppError::TimedOut("shutdown.exe".into())),
            CommandOutcome::Cancelled => Err(AppError::Cancelled),
        }
    }

    async fn cancel_restart(&self) -> AppResult<()> {
        let exe = self.inner.system32("shutdown.exe")?;
        let r = self.inner.executor.run(CommandSpec { program: exe, args: vec!["/a".into()], timeout: Duration::from_secs(30) }, &NullSink, &CancellationToken::new()).await?;
        match r.outcome {
            CommandOutcome::Exited(0) => {
                self.inner.audit(ActivityKind::RestartCancelled, &ToolId::new("setup-restart").expect("slug"), "scheduled restart cancelled");
                Ok(())
            }
            CommandOutcome::Exited(_) => Err(AppError::Validation("no restart is scheduled".into())),
            other => Err(AppError::Io(format!("{other:?}"))),
        }
    }
}

struct NullSink;
impl LineSink for NullSink {
    fn line(&self, _: OutputStream, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::activity_sqlite::SqliteActivityLogger;
    use crate::adapters::sqlite_store::SqliteJobStore;
    use crate::adapters::testing::{MockExecutor, MockLocator, Step};
    use crate::domain::platform::fixtures::{ready_facts, this_pc_facts};
    use crate::domain::platform::{DistroState, WslDistro};
    use crate::ports::ElevatedOutcome;
    use std::sync::Mutex;

    /// A detector that returns scripted reports in order (the last one repeats).
    struct Script(Mutex<Vec<PlatformReport>>);
    #[async_trait]
    impl PlatformDetector for Script {
        async fn detect(&self) -> PlatformReport {
            let mut v = self.0.lock().unwrap();
            if v.len() > 1 { v.remove(0) } else { v[0].clone() }
        }
    }

    struct FakeElevated {
        outcome: Mutex<ElevatedOutcome>,
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    }
    #[async_trait]
    impl ElevatedRunner for FakeElevated {
        async fn run(&self, program: &std::path::Path, args: &[String], _: Duration) -> AppResult<ElevatedOutcome> {
            self.calls.lock().unwrap().push((program.to_path_buf(), args.to_vec()));
            Ok(self.outcome.lock().unwrap().clone())
        }
    }

    fn report(f: crate::domain::platform::Facts) -> PlatformReport {
        PlatformReport::from_facts(f)
    }

    fn with_distro() -> PlatformReport {
        report(ready_facts())
    }

    fn engine(reports: Vec<PlatformReport>, elevated: ElevatedOutcome, exec: Arc<MockExecutor>) -> (SetupEngine, Arc<FakeElevated>, Arc<SqliteJobStore>, Arc<SqliteActivityLogger>) {
        let el = Arc::new(FakeElevated { outcome: Mutex::new(elevated), calls: Mutex::new(vec![]) });
        let store = Arc::new(SqliteJobStore::in_memory().unwrap());
        let act = Arc::new(SqliteActivityLogger::in_memory().unwrap());
        let mut loc = MockLocator::default();
        loc.add("wsl.exe", "C:\\Windows\\System32\\wsl.exe");
        loc.add("shutdown.exe", "C:\\Windows\\System32\\shutdown.exe");
        let e = SetupEngine::new(Arc::new(Script(Mutex::new(reports))), exec, el.clone(), Arc::new(loc), store.clone(), act.clone());
        (e, el, store, act)
    }

    fn noop_exec() -> Arc<MockExecutor> {
        Arc::new(MockExecutor::new(|_| Step::ok(&[])))
    }

    const WAIT: Duration = Duration::from_secs(10);

    #[tokio::test]
    async fn nothing_changes_without_explicit_confirmation() {
        let (e, el, store, _) = engine(vec![report(this_pc_facts())], ElevatedOutcome { exit_code: Some(0), declined: false }, noop_exec());
        assert!(matches!(e.run_action(SetupActionId::InstallWsl, false).await, Err(AppError::Validation(_))));
        assert!(el.calls.lock().unwrap().is_empty());
        assert!(store.list_jobs(10).unwrap().is_empty());
        assert!(matches!(e.request_restart(false, 60).await, Err(AppError::Validation(_))));
    }

    #[tokio::test]
    async fn manual_steps_are_never_executed() {
        let (e, el, _, _) = engine(vec![report(this_pc_facts())], ElevatedOutcome { exit_code: Some(0), declined: false }, noop_exec());
        for id in [SetupActionId::EnableVirtualization, SetupActionId::InstallDockerDesktop, SetupActionId::StartDockerDesktop, SetupActionId::FreeDiskSpace, SetupActionId::InstallWinget, SetupActionId::ConvertDistroToWsl2] {
            assert!(matches!(e.run_action(id, true).await, Err(AppError::Unsupported(_))), "{id:?}");
        }
        assert!(el.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn installing_wsl_runs_one_fixed_elevated_command_and_reports_the_pending_restart() {
        let mut pending = this_pc_facts();
        pending.system.pending_restart = Some(true);
        // Reports: before (needs WSL), then after (still not detected but restart pending).
        let (e, el, store, act) = engine(vec![report(this_pc_facts()), report(pending)], ElevatedOutcome { exit_code: Some(0), declined: false }, noop_exec());
        let op = e.run_action(SetupActionId::InstallWsl, true).await.unwrap();
        let job = e.wait(op, WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Succeeded, "{job:?}");
        assert!(job.message.contains("Restart Windows") && job.message.contains("without your confirmation"));
        let calls = el.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, ["--install", "--no-distribution"]);
        assert!(calls[0].0.ends_with("wsl.exe"));
        let out = store.job_output(op, 10).unwrap().join("\n");
        assert!(out.contains("(administrator) wsl.exe --install --no-distribution"));
        let kinds: Vec<_> = act.recent(10).unwrap().into_iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&ActivityKind::SetupStarted) && kinds.contains(&ActivityKind::SetupSucceeded));
        // No restart was ever requested by installing.
        assert!(!kinds.contains(&ActivityKind::RestartRequested));
    }

    #[tokio::test]
    async fn declining_the_uac_prompt_changes_nothing_and_says_so() {
        let (e, _, _, _) = engine(vec![report(this_pc_facts())], ElevatedOutcome { exit_code: None, declined: true }, noop_exec());
        let op = e.run_action(SetupActionId::InstallWsl, true).await.unwrap();
        let job = e.wait(op, WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Failed);
        let err = job.error.unwrap();
        assert_eq!(err.code, "missing_permissions");
        assert!(err.message.contains("declined"));
    }

    #[tokio::test]
    async fn a_command_that_succeeds_but_changes_nothing_is_not_reported_as_success() {
        let (e, _, _, _) = engine(vec![report(this_pc_facts())], ElevatedOutcome { exit_code: Some(0), declined: false }, noop_exec());
        let job = e.wait(e.run_action(SetupActionId::InstallWsl, true).await.unwrap(), WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Failed);
        assert_eq!(job.error.unwrap().code, "verification_failed");
    }

    #[tokio::test]
    async fn nonzero_elevated_exit_codes_are_reported_with_a_troubleshooting_link() {
        let (e, _, _, _) = engine(vec![report(this_pc_facts())], ElevatedOutcome { exit_code: Some(0x80370102_u32 as i32), declined: false }, noop_exec());
        let job = e.wait(e.run_action(SetupActionId::InstallWsl, true).await.unwrap(), WAIT).await.unwrap();
        assert_eq!(job.error.as_ref().unwrap().code, "command_failed");
        assert!(job.error.unwrap().message.contains("0x80370102") && job.exit_code == Some(0x80370102_u32 as i32));
    }

    #[tokio::test]
    async fn distro_install_requires_wsl_first_and_verifies_the_distribution() {
        // WSL itself missing: the distro step is refused until WSL (and the restart) is done.
        let (e, _, _, _) = engine(vec![report(this_pc_facts())], ElevatedOutcome { exit_code: Some(0), declined: false }, noop_exec());
        let job = e.wait(e.run_action(SetupActionId::InstallDistro, true).await.unwrap(), WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Failed);
        assert_eq!(job.error.unwrap().code, "prerequisite_missing");

        // WSL present, no distro; the command runs unelevated; the re-check finds the distro.
        let mut no_distro = ready_facts();
        no_distro.wsl.distros.clear();
        let exec = Arc::new(MockExecutor::new(|c| {
            assert_eq!(c.args, ["--install", "--distribution", "Ubuntu-24.04", "--no-launch"]);
            Step::ok(&["Downloading: Ubuntu 24.04 LTS", "Installing: Ubuntu 24.04 LTS", "Distribution successfully installed."])
        }));
        let (e, el, store, _) = engine(vec![report(no_distro), with_distro()], ElevatedOutcome { exit_code: Some(0), declined: false }, exec);
        let op = e.run_action(SetupActionId::InstallDistro, true).await.unwrap();
        let job = e.wait(op, WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Succeeded, "{job:?}");
        assert!(el.calls.lock().unwrap().is_empty(), "installing a distribution needs no administrator rights");
        assert!(store.job_output(op, 20).unwrap().iter().any(|l| l.contains("successfully installed")));
    }

    #[tokio::test]
    async fn distro_install_failures_are_diagnosed() {
        let mut no_distro = ready_facts();
        no_distro.wsl.distros.clear();
        let exec = Arc::new(MockExecutor::new(|_| Step::exit(-1, &["WSL2 is not supported with your current machine configuration.", "Error code: Wsl/InstallDistro/Service/RegisterDistro/CreateVm/HCS/HCS_E_HYPERV_NOT_INSTALLED"])));
        let (e, _, _, _) = engine(vec![report(no_distro)], ElevatedOutcome { exit_code: Some(0), declined: false }, exec);
        let job = e.wait(e.run_action(SetupActionId::InstallDistro, true).await.unwrap(), WAIT).await.unwrap();
        assert_eq!(job.error.unwrap().code, "virtualization_disabled");
    }

    #[tokio::test]
    async fn an_already_satisfied_step_is_refused_and_steps_do_not_overlap() {
        let (e, _, _, _) = engine(vec![report(ready_facts())], ElevatedOutcome { exit_code: Some(0), declined: false }, noop_exec());
        let job = e.wait(e.run_action(SetupActionId::InstallWsl, true).await.unwrap(), WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Failed);
        assert_eq!(job.error.unwrap().code, "validation");

        let slow = Arc::new(MockExecutor::new(|_| Step::ok(&[]).delayed(400)));
        let mut no_distro = ready_facts();
        no_distro.wsl.distros.clear();
        let (e, _, _, _) = engine(vec![report(no_distro.clone()), with_distro()], ElevatedOutcome { exit_code: Some(0), declined: false }, slow);
        let first = e.run_action(SetupActionId::InstallDistro, true).await.unwrap();
        assert!(matches!(e.run_action(SetupActionId::InstallDistro, true).await, Err(AppError::Busy(_))));
        assert!(matches!(e.run_action(SetupActionId::InstallWsl, true).await, Err(AppError::Busy(_))));
        e.wait(first, WAIT).await.unwrap();
        let _ = (DistroState::Running, WslDistro { name: String::new(), state: DistroState::Running, version: None, is_default: false, internal: false, os_id: None });
    }

    #[tokio::test]
    async fn restart_is_scheduled_only_after_confirmation_with_a_cancellable_delay() {
        let seen = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let s2 = seen.clone();
        let exec = Arc::new(MockExecutor::new(move |c| {
            s2.lock().unwrap().push(c.args.clone());
            Step::ok(&[])
        }));
        let (e, _, _, act) = engine(vec![report(this_pc_facts())], ElevatedOutcome { exit_code: Some(0), declined: false }, exec);
        e.request_restart(true, 5).await.unwrap(); // delay is clamped up to 30 s
        e.cancel_restart().await.unwrap();
        let calls = seen.lock().unwrap().clone();
        assert_eq!(calls[0][..3], ["/r", "/t", "30"]);
        assert!(calls[0].contains(&"/c".to_string()));
        assert_eq!(calls[1], ["/a"]);
        let kinds: Vec<_> = act.recent(10).unwrap().into_iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&ActivityKind::RestartRequested) && kinds.contains(&ActivityKind::RestartCancelled));
        assert!(calls.iter().all(|a| !a.contains(&"/f".to_string())), "never force-close applications");
    }
}

