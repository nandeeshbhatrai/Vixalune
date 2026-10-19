//! Launching container tools. The only launch path implemented so far.
//!
//! A run is a tracked job (state, streamed output, cancellation, audit). Before it starts, every
//! approval is validated again against the *canonical* host path (so junctions cannot smuggle in a
//! protected folder), the tool's image must already be installed (`--pull never`), and the engine
//! must be local, running and Linux. Cancelling or timing out removes the named container, not
//! just the docker client process.

use crate::adapters::container::{docker, image_for, ContainerAdapter};
use crate::domain::activity::{ActivityEvent, ActivityKind};
use crate::domain::container::{build_docker_run, container_name, validate_host_mount, DockerRun, RunApprovals};
use crate::domain::jobs::{Job, JobError, JobEvent, JobKind, JobPhase};
use crate::domain::manifest::{build_command, InstallSpec, ParamValue, RunSpec, RuntimeKind, ToolManifest};
use crate::adapters::run_support::RunSupport;
use crate::domain::process::{CancellationToken, OutputStream};
use crate::domain::runs::{classify_outcome, RunOutcome, RunRecord};
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::{
    ActivityLogger, AdapterContext, CommandExecutor, CommandOutcome, CommandSpec, ExecutableLocator, InstallerAdapter, JobStore, LineSink, PresenceStatus,
    RunPreview, ToolRegistry, ToolRunner,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RUN_TIMEOUT_SECS: u32 = 2 * 60 * 60;
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ContainerToolRunner {
    inner: Arc<Inner>,
}

struct Inner {
    registry: Arc<dyn ToolRegistry>,
    executor: Arc<dyn CommandExecutor>,
    locator: Arc<dyn ExecutableLocator>,
    adapter: Arc<ContainerAdapter>,
    store: Arc<dyn JobStore>,
    activity: Arc<dyn ActivityLogger>,
    approved_registries: Vec<String>,
    home: Option<PathBuf>,
    running: Mutex<HashMap<OperationId, CancellationToken>>,
    /// Run history and lab placeholders. Optional so the engine-level tests can use the runner on its own.
    support: Option<Arc<RunSupport>>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

struct JobSink {
    store: Arc<dyn JobStore>,
    id: OperationId,
}
impl LineSink for JobSink {
    fn line(&self, stream: OutputStream, text: &str) {
        let _ = self.store.append_output(self.id, stream, text);
    }
}

/// Resolve junctions/symlinks, then validate the *real* folder. `\\?\C:\x` becomes `C:\x`; UNC targets are refused.
pub fn canonical_mount(raw: &str, home: Option<&Path>) -> AppResult<String> {
    let lexical = validate_host_mount(raw, home)?;
    let canon = std::fs::canonicalize(&lexical).map_err(|e| AppError::Validation(format!("the folder '{raw}' cannot be used: {e}")))?;
    if !canon.is_dir() {
        return Err(AppError::Validation(format!("'{raw}' is not a folder")));
    }
    let s = canon.to_string_lossy().to_string();
    let plain = if let Some(rest) = s.strip_prefix("\\\\?\\UNC\\") {
        return Err(AppError::Validation(format!("'{raw}' resolves to a network path (\\\\{rest}); network folders cannot be mounted")));
    } else if let Some(rest) = s.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        s
    };
    validate_host_mount(&plain, home)
}

impl ContainerToolRunner {
    pub fn new(
        registry: Arc<dyn ToolRegistry>,
        executor: Arc<dyn CommandExecutor>,
        locator: Arc<dyn ExecutableLocator>,
        adapter: Arc<ContainerAdapter>,
        store: Arc<dyn JobStore>,
        activity: Arc<dyn ActivityLogger>,
        approved_registries: Vec<String>,
        home: Option<PathBuf>,
    ) -> Self {
        Self { inner: Arc::new(Inner { registry, executor, locator, adapter, store, activity, approved_registries, home, running: Mutex::new(HashMap::new()), support: None }) }
    }

    /// Attach run history and lab placeholders (call right after `new`).
    pub fn with_support(mut self, support: Arc<RunSupport>) -> Self {
        Arc::get_mut(&mut self.inner).expect("the runner is not shared yet").support = Some(support);
        self
    }

    /// Cancel a running container job. `NotFound` if this runner does not own it.
    pub async fn cancel(&self, op: OperationId) -> AppResult<()> {
        let running = self.inner.running.lock().unwrap();
        let token = running.get(&op).ok_or_else(|| AppError::NotFound(format!("running container job '{op}'")))?;
        token.cancel();
        Ok(())
    }

    /// Number of runs that are currently active.
    pub fn active_count(&self) -> usize {
        self.inner.running.lock().unwrap().len()
    }

    /// Ask every active run to stop (application shutdown). Returns how many were asked.
    pub fn cancel_all(&self) -> usize {
        let running = self.inner.running.lock().unwrap();
        running.values().for_each(|t| t.cancel());
        running.len()
    }

    pub async fn wait(&self, op: OperationId, timeout: Duration) -> AppResult<Job> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let j = self.inner.store.get_job(op)?;
            if j.phase.is_terminal() {
                return Ok(j);
            }
            if tokio::time::Instant::now() > deadline {
                return Err(AppError::TimedOut("container job".into()));
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
}

impl Inner {
    fn resolve(&self, id: &ToolId, profile: &str) -> AppResult<(ToolManifest, RunSpec, InstallSpec)> {
        let m = self.registry.get(id)?.manifest;
        let p = m.profile(profile).cloned().ok_or_else(|| AppError::NotFound(format!("launch profile '{profile}' of '{id}'")))?;
        if p.runtime != RuntimeKind::Container {
            return Err(AppError::NotImplemented("launching tools outside containers"));
        }
        let spec = m.install.iter().find(|s| matches!(s, InstallSpec::ContainerImage { .. })).cloned().ok_or_else(|| AppError::Validation("this tool has no container image".into()))?;
        Ok((m, p, spec))
    }

    fn plan(&self, m: &ToolManifest, p: &RunSpec, spec: &InstallSpec, values: &HashMap<String, ParamValue>, approvals: &RunApprovals, name: &str) -> AppResult<(crate::domain::manifest::CommandPreview, DockerRun)> {
        let cmd = match &self.support {
            Some(s) => s.build(p, values, &s.run_dir(OperationId::new()))?.preview,
            None => build_command(p, values)?,
        };
        let img = image_for(m, spec, &self.approved_registries)?;
        let run = build_docker_run(m, p, &cmd.args, &img.reference(), approvals, name, self.home.as_deref())?;
        Ok((cmd, run))
    }

    fn audit(&self, kind: ActivityKind, tool: &ToolId, s: &str) {
        let _ = self.activity.record(ActivityEvent { at: now(), kind, tool_id: Some(tool.clone()), summary: s.into() });
    }

    fn step(&self, job: &mut Job, ev: JobEvent, msg: &str) -> AppResult<()> {
        job.phase = job.phase.apply(ev)?;
        job.message = msg.to_string();
        job.updated_at = now();
        if job.phase.is_terminal() {
            job.finished_at = Some(job.updated_at.clone());
        }
        self.store.update_job(job)
    }

    async fn remove_container(&self, docker_exe: &Path, name: &str) {
        let _ = self
            .executor
            .run(CommandSpec { program: docker_exe.to_path_buf(), args: docker::force_remove(name), timeout: READ_TIMEOUT }, &NullSink, &CancellationToken::new())
            .await;
    }
}

struct NullSink;
impl LineSink for NullSink {
    fn line(&self, _: OutputStream, _: &str) {}
}

#[async_trait]
impl ToolRunner for ContainerToolRunner {
    async fn preview(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &RunApprovals) -> AppResult<RunPreview> {
        let (m, p, spec) = self.inner.resolve(id, profile)?;
        // Lexical validation only: previewing must not touch the file system or the engine.
        let (command, run) = self.inner.plan(&m, &p, &spec, values, approvals, "vixalune-preview")?;
        Ok(RunPreview {
            command,
            container: Some(run.summary),
            scope: None,
            network_activity: p.network_activity.clone(),
            timeout_secs: p.timeout_secs.unwrap_or(600),
            output_format: p.output_format.clone(),
            expected_output: p.expected_output.clone(),
            working_dir: "container".into(),
            notes: vec![],
        })
    }

    async fn start(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &RunApprovals) -> AppResult<OperationId> {
        let inner = &self.inner;
        let (m, p, spec) = inner.resolve(id, profile)?;

        // Re-validate every approved folder against its real location.
        let mut real = approvals.clone();
        for (k, v) in &approvals.mounts {
            real.mounts.insert(k.clone(), canonical_mount(v, inner.home.as_deref())?);
        }
        let name = container_name(id.as_str(), &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let (cmd, run) = inner.plan(&m, &p, &spec, values, &real, &name)?;

        let docker_exe = inner.locator.locate("docker.exe", &[]).ok_or_else(|| AppError::PrerequisiteMissing("docker.exe was not found".into()))?;
        let cancel = CancellationToken::new();
        let sink = JobSink { store: inner.store.clone(), id: OperationId::new() };
        let ctx = AdapterContext { tool: &m, cancel: &cancel, sink: &sink };
        // The image must be installed already: runs never pull.
        match inner.adapter.status(&ctx, &spec).await? {
            PresenceStatus::Installed { .. } => {}
            PresenceStatus::NotInstalled => return Err(AppError::PrerequisiteMissing(format!("install {} first; its image has not been downloaded", m.name))),
            PresenceStatus::Unknown(why) => return Err(AppError::Environment(why)),
        }
        inner.adapter.ensure_ready(&ctx).await?;

        let ts = now();
        let mut job = Job {
            id: OperationId::new(),
            tool_id: id.clone(),
            kind: JobKind::Run,
            phase: JobPhase::Queued,
            message: "Queued".into(),
            percent: None,
            started_at: ts.clone(),
            updated_at: ts,
            finished_at: None,
            exit_code: None,
            allow_elevation: false,
            error: None,
        };
        inner.store.create_job(&job)?; // Busy if the tool is installing or already running.
        inner.running.lock().unwrap().insert(job.id, cancel.clone());
        inner.audit(ActivityKind::RunStarted, id, &format!("{} profile '{}' (network: {}, mounts: {}, ports: {})", m.name, p.id, run.summary.network, run.summary.mounts.len(), run.summary.ports.len()));
        if let Some(s) = &inner.support {
            s.record_start(&RunRecord {
                id: job.id,
                tool_id: id.clone(),
                tool_name: m.name.clone(),
                profile_id: p.id.clone(),
                profile_label: p.label.clone(),
                tool_version: inner.store.get_installation(id).ok().flatten().and_then(|r| r.version),
                started_at: job.started_at.clone(),
                finished_at: None,
                duration_ms: None,
                exit_code: None,
                outcome: None,
                command: crate::domain::activity::redact(&cmd.display),
                targets: vec![],
                scope: if run.summary.network == "none" { "no network access".into() } else { "network access approved by you".into() },
                network_activity: p.network_activity,
                output_format: p.output_format,
                run_dir: None,
                result_path: None,
            });
        }

        let inner = self.inner.clone();
        let op = job.id;
        let display = cmd.display.clone();
        let timeout = Duration::from_secs(u64::from(p.timeout_secs.unwrap_or(RUN_TIMEOUT_SECS)));
        tokio::spawn(async move {
            let sink = JobSink { store: inner.store.clone(), id: op };
            let mut finished: (Option<i32>, RunOutcome) = (None, RunOutcome::Failed);
            let finish = async {
                inner.step(&mut job, JobEvent::StartChecks, "Starting")?;
                inner.step(&mut job, JobEvent::BeginRun, &format!("Running {display} in a container"))?;
                let _ = inner.store.append_output(op, OutputStream::Stdout, &format!("> docker {}", run.args.join(" ")));
                let r = inner.executor.run(CommandSpec { program: docker_exe.clone(), args: run.args.clone(), timeout }, &sink, &cancel).await?;
                match r.outcome {
                    CommandOutcome::Exited(code) => {
                        let output = inner.store.job_output(op, 100_000).map(|l| l.join("\n")).unwrap_or_default();
                        let outcome = classify_outcome(Some(code), false, false, &p.exit_codes, &p.findings_patterns, &output).unwrap_or(RunOutcome::Failed);
                        finished = (Some(code), outcome);
                        job.exit_code = Some(code);
                        if outcome.is_success() {
                            inner.step(&mut job, JobEvent::RunDone, if outcome == RunOutcome::Findings { "Finished: the tool reported findings" } else { "Finished: nothing reported" })?;
                            inner.audit(ActivityKind::RunFinished, &job.tool_id, &format!("{outcome:?}, exit code {code}"));
                        } else {
                            let e = AppError::CommandFailed { code: Some(code), message: format!("the tool exited with code {code}, which this profile does not treat as success") };
                            job.error = Some(JobError::from_app(&e, Some(r.tail.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join(" | "))));
                            inner.step(&mut job, JobEvent::Fail, &e.to_string())?;
                            inner.audit(ActivityKind::RunFinished, &job.tool_id, &format!("exit code {code}"));
                        }
                    }
                    CommandOutcome::TimedOut => {
                        finished = (None, RunOutcome::TimedOut);
                        inner.remove_container(&docker_exe, &run.container_name).await;
                        let e = AppError::TimedOut(format!("the run exceeded {} seconds and was stopped", timeout.as_secs()));
                        job.error = Some(JobError::from_app(&e, None));
                        inner.step(&mut job, JobEvent::Fail, &e.to_string())?;
                        inner.audit(ActivityKind::RunTerminated, &job.tool_id, "timed out");
                    }
                    CommandOutcome::Cancelled => return Err(AppError::Cancelled),
                }
                Ok::<(), AppError>(())
            };
            if let Err(e) = finish.await {
                match e {
                    AppError::Cancelled => {
                        finished = (None, RunOutcome::Cancelled);
                        // Killing the docker client does not stop the container: remove it by name.
                        inner.remove_container(&docker_exe, &run.container_name).await;
                        let _ = inner.step(&mut job, JobEvent::RequestCancel, "Cancelling");
                        let _ = inner.step(&mut job, JobEvent::CancelDone, "Cancelled; the container was removed");
                        inner.audit(ActivityKind::RunTerminated, &job.tool_id, "cancelled by the user");
                    }
                    other => {
                        job.error = Some(JobError::from_app(&other, None));
                        if inner.step(&mut job, JobEvent::Fail, &other.to_string()).is_err() {
                            job.phase = JobPhase::Failed;
                            job.finished_at = Some(now());
                            let _ = inner.store.update_job(&job);
                        }
                        inner.audit(ActivityKind::RunTerminated, &job.tool_id, other.code());
                    }
                }
            }
            if let Some(s) = &inner.support {
                s.record_finish(op, finished.0, finished.1);
            }
            inner.running.lock().unwrap().remove(&op);
        });
        Ok(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::activity_sqlite::SqliteActivityLogger;
    use crate::adapters::registry::ManifestRegistry;
    use crate::adapters::sqlite_store::SqliteJobStore;
    use crate::adapters::testing::{MockExecutor, MockLocator, Step};
    use crate::ports::TrustLevel;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Fixture {
        runner: ContainerToolRunner,
        exec: Arc<MockExecutor>,
        store: Arc<SqliteJobStore>,
        sandbox: PathBuf,
        _dir: tempfile::TempDir,
    }

    /// A folder with a name the validator accepts (the temp dir lives under AppData, which it refuses).
    fn project_dir(name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("mount-tests").join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn fixture(image_present: bool, run: Step) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mdir = dir.path().join("manifests");
        std::fs::create_dir_all(&mdir).unwrap();
        std::fs::write(mdir.join("semgrep.toml"), include_str!("../../../manifests/semgrep.toml")).unwrap();
        let registry = Arc::new(ManifestRegistry::load_dir(&mdir, TrustLevel::Bundled).unwrap());
        let present = Arc::new(AtomicBool::new(image_present));
        let run2 = run.clone();
        let exec = Arc::new(MockExecutor::new(move |c| match c.args[0].as_str() {
            "version" => Step::ok(&["client=29.8.2 context=desktop-linux server=29.8.2 linux amd64"]),
            "context" => Step::ok(&["npipe:////./pipe/dockerDesktopLinuxEngine"]),
            "image" => {
                if present.load(Ordering::SeqCst) {
                    Step::ok(&["sha256:0123456789abcdef linux amd64"])
                } else {
                    Step::exit(1, &["Error response from daemon: No such image"])
                }
            }
            "run" => {
                assert!(crate::domain::container::assert_hardened(&c.args).is_ok());
                run2.clone()
            }
            "rm" => Step::ok(&[]),
            other => panic!("unexpected {other}"),
        }));
        let mut loc = MockLocator::default();
        loc.add("docker.exe", "C:\\fake\\docker.exe");
        let loc = Arc::new(loc);
        let regs = vec!["docker.io".to_string()];
        let home = Some(PathBuf::from("C:\\Users\\me"));
        let adapter = Arc::new(ContainerAdapter::new(exec.clone(), loc.clone(), regs.clone(), home.clone()));
        let store = Arc::new(SqliteJobStore::in_memory().unwrap());
        let runner = ContainerToolRunner::new(registry, exec.clone(), loc, adapter, store.clone(), Arc::new(SqliteActivityLogger::in_memory().unwrap()), regs, home);
        Fixture { runner, exec, store, sandbox: project_dir("proj"), _dir: dir }
    }

    fn semgrep() -> ToolId {
        ToolId::new("semgrep").unwrap()
    }

    fn approvals(path: &Path, network: bool) -> RunApprovals {
        RunApprovals { network, mounts: HashMap::from([("source".to_string(), path.display().to_string())]), ports: HashMap::new(), scope: None }
    }

    const WAIT: Duration = Duration::from_secs(10);

    #[tokio::test]
    async fn preview_shows_exactly_what_will_run_without_touching_the_engine() {
        let f = fixture(true, Step::ok(&[]));
        let pv = f.runner.preview(&semgrep(), "scan-registry-rules", &HashMap::new(), &approvals(&f.sandbox, true)).await.unwrap();
        let c = pv.container.unwrap();
        assert_eq!(c.network, "bridge");
        assert!(c.mounts[0].read_only && c.mounts[0].container == "/src");
        assert!(c.warnings.iter().any(|w| w.contains("internet")));
        assert_eq!(c.user, "65532:65532");
        assert!(pv.command.display.starts_with("semgrep scan --config p/default"));
        assert!(f.exec.calls().is_empty());
        // Not approving the network is refused at preview time.
        assert!(f.runner.preview(&semgrep(), "scan-registry-rules", &HashMap::new(), &approvals(&f.sandbox, false)).await.is_err());
    }

    #[tokio::test]
    async fn a_run_is_a_tracked_hardened_job_with_streamed_output() {
        let f = fixture(true, Step::ok(&["Ran 0 rules on 3 files: 0 findings."]));
        let vals = HashMap::from([("config".to_string(), ParamValue::Text("/src/.semgrep.yml".into()))]);
        let op = f.runner.start(&semgrep(), "scan-local", &vals, &approvals(&f.sandbox, false)).await.unwrap();
        let job = f.runner.wait(op, WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Succeeded, "{job:?}");
        assert_eq!(job.exit_code, Some(0));
        let out = f.store.job_output(op, 20).unwrap();
        assert!(out[0].starts_with("> docker run --rm --pull never"));
        assert!(out.iter().any(|l| l.contains("0 findings")));
        let run = f.exec.calls().into_iter().find(|c| c.args[0] == "run").unwrap();
        assert!(run.args.windows(2).any(|w| w[0] == "--network" && w[1] == "none"));
        assert!(run.args.iter().any(|a| a.starts_with("type=bind,source=") && a.ends_with(",readonly")));
        assert!(run.args.contains(&"--config".to_string()) && run.args.contains(&"/src/.semgrep.yml".to_string()));
        // The tool cannot be started twice at once, and a finished run frees it.
        assert!(f.store.active_job_for(&semgrep()).unwrap().is_none());
    }

    #[tokio::test]
    async fn runs_are_refused_without_the_image_or_with_unsafe_approvals() {
        let f = fixture(false, Step::ok(&[]));
        let err = f.runner.start(&semgrep(), "scan-registry-rules", &HashMap::new(), &approvals(&f.sandbox, true)).await.unwrap_err();
        assert!(matches!(err, AppError::PrerequisiteMissing(_)), "{err:?}");
        assert!(f.exec.calls().iter().all(|c| c.args[0] != "run"), "never pulls or runs without an installed image");

        let f = fixture(true, Step::ok(&[]));
        for evil in ["C:\\", "C:\\Windows\\System32", "C:\\Users", "\\\\server\\share", "D:\\does\\not\\exist"] {
            let a = RunApprovals { network: true, mounts: HashMap::from([("source".to_string(), evil.to_string())]), ports: HashMap::new(), scope: None };
            assert!(f.runner.start(&semgrep(), "scan-registry-rules", &HashMap::new(), &a).await.is_err(), "{evil}");
        }
        assert!(f.runner.start(&semgrep(), "scan-registry-rules", &HashMap::new(), &approvals(&f.sandbox, false)).await.is_err(), "network not approved");
        assert!(f.exec.calls().iter().all(|c| c.args[0] != "run"));
        assert!(f.runner.start(&semgrep(), "no-such-profile", &HashMap::new(), &RunApprovals::default()).await.is_err());
    }

    #[tokio::test]
    async fn junctions_to_protected_folders_are_resolved_and_refused() {
        let f = fixture(true, Step::ok(&[]));
        let link = project_dir("junctions").join("to-windows");
        let _ = std::fs::remove_dir(&link);
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J", &link.display().to_string(), "C:\\Windows"]).output();
        if made.map(|o| o.status.success()).unwrap_or(false) {
            let err = f.runner.start(&semgrep(), "scan-registry-rules", &HashMap::new(), &approvals(&link, true)).await.unwrap_err();
            assert!(err.to_string().contains("protected system folder"), "{err}");
            let _ = std::fs::remove_dir(&link);
        }
    }

    #[tokio::test]
    async fn non_zero_exit_and_cancellation_are_reported_and_the_container_is_removed() {
        let f = fixture(true, Step::exit(2, &["error: invalid rules file"]));
        let vals = HashMap::from([("config".to_string(), ParamValue::Text("/src/.semgrep.yml".into()))]);
        let op = f.runner.start(&semgrep(), "scan-local", &vals, &approvals(&f.sandbox, false)).await.unwrap();
        let job = f.runner.wait(op, WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Failed);
        assert_eq!(job.exit_code, Some(2));
        assert!(job.error.unwrap().detail.unwrap().contains("invalid rules file"));

        let f = fixture(true, Step::hang(&["scanning..."]));
        let op = f.runner.start(&semgrep(), "scan-local", &vals, &approvals(&f.sandbox, false)).await.unwrap();
        for _ in 0..200 {
            if f.store.get_job(op).unwrap().phase == JobPhase::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(f.runner.start(&semgrep(), "scan-local", &vals, &approvals(&f.sandbox, false)).await.is_err(), "no second run while one is active");
        f.runner.cancel(op).await.unwrap();
        let job = f.runner.wait(op, WAIT).await.unwrap();
        assert_eq!(job.phase, JobPhase::Cancelled);
        let name = f.exec.calls().into_iter().find(|c| c.args[0] == "run").map(|c| c.args[c.args.iter().position(|a| a == "--name").unwrap() + 1].clone()).unwrap();
        let rm = f.exec.calls().into_iter().find(|c| c.args[0] == "rm").expect("the container must be removed by name");
        assert_eq!(rm.args, ["rm", "-f", name.as_str()]);
        assert!(f.runner.cancel(op).await.is_err());
    }

    #[tokio::test]
    async fn only_container_profiles_can_be_launched_for_now() {
        let f = fixture(true, Step::ok(&[]));
        let nmap = ToolId::new("nmap").unwrap();
        assert!(matches!(f.runner.preview(&nmap, "scan", &HashMap::new(), &RunApprovals::default()).await, Err(AppError::NotFound(_))));
    }
}
