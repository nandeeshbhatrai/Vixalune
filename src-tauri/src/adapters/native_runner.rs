//! Launching Windows-native tools (Nmap, Trivy, ZAP, ...).
//!
//! A run is a tracked job. Everything that reaches the process was validated first: parameters are typed and
//! range-checked, the argument array is built from the profile (never from text), targets pass the scope engine
//! (again at start time, with a fresh name resolution), file parameters are checked against the real file system,
//! and the program is an absolute path found by the locator. The child runs unelevated, in its own empty run folder,
//! with a scrubbed environment, and is killed (with its children) on timeout or cancellation.

use crate::domain::activity::{redact, ActivityEvent, ActivityKind};
use crate::domain::jobs::{Job, JobError, JobEvent, JobKind, JobPhase};
use crate::domain::manifest::{ParamValue, RunSpec, RuntimeKind, ToolManifest, WorkingDir};
use crate::domain::paths::resolve_location;
use crate::domain::process::{CancellationToken, OutputStream};
use crate::domain::runs::{classify_outcome, RunOutcome, RunRecord};
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::adapters::run_support::{now, RunSupport};
use crate::ports::{
    ActivityLogger, CommandExecutor, CommandOutcome, CommandSpec, ExecOptions, ExecutableLocator, JobStore, LineSink, RunPreview, ToolRegistry, ToolRunner,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u32 = 600;

pub struct NativeToolRunner {
    inner: Arc<Inner>,
}

struct Inner {
    registry: Arc<dyn ToolRegistry>,
    executor: Arc<dyn CommandExecutor>,
    locator: Arc<dyn ExecutableLocator>,
    store: Arc<dyn JobStore>,
    activity: Arc<dyn ActivityLogger>,
    support: Arc<RunSupport>,
    home: Option<PathBuf>,
    running: Mutex<HashMap<OperationId, CancellationToken>>,
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

impl NativeToolRunner {
    pub fn new(
        registry: Arc<dyn ToolRegistry>,
        executor: Arc<dyn CommandExecutor>,
        locator: Arc<dyn ExecutableLocator>,
        store: Arc<dyn JobStore>,
        activity: Arc<dyn ActivityLogger>,
        support: Arc<RunSupport>,
        home: Option<PathBuf>,
    ) -> Self {
        Self { inner: Arc::new(Inner { registry, executor, locator, store, activity, support, home, running: Mutex::new(HashMap::new()) }) }
    }

    /// Stop a running job. `NotFound` if this runner does not own it.
    pub async fn cancel(&self, op: OperationId) -> AppResult<()> {
        let running = self.inner.running.lock().unwrap();
        running.get(&op).ok_or_else(|| AppError::NotFound(format!("running job '{op}'")))?.cancel();
        Ok(())
    }

    pub async fn wait(&self, op: OperationId, timeout: Duration) -> AppResult<Job> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let j = self.inner.store.get_job(op)?;
            if j.phase.is_terminal() {
                return Ok(j);
            }
            if tokio::time::Instant::now() > deadline {
                return Err(AppError::TimedOut("native job".into()));
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
}

impl Inner {
    fn resolve(&self, id: &ToolId, profile: &str) -> AppResult<(ToolManifest, RunSpec)> {
        let m = self.registry.get(id)?.manifest;
        let p = m.profile(profile).cloned().ok_or_else(|| AppError::NotFound(format!("launch profile '{profile}' of '{id}'")))?;
        match p.runtime {
            RuntimeKind::WindowsNative => Ok((m, p)),
            RuntimeKind::Wsl => Err(AppError::NotImplemented("launching tools inside WSL")),
            RuntimeKind::Container => Err(AppError::Validation("this profile runs in a container, not natively".into())),
        }
    }

    /// The executable: install folder recorded by Vixalune, the profile's known folders, then the system PATH.
    fn locate(&self, m: &ToolManifest, p: &RunSpec) -> AppResult<PathBuf> {
        let mut extra: Vec<PathBuf> = Vec::new();
        if let Ok(Some(rec)) = self.store.get_installation(&m.id) {
            if let Some(loc) = rec.location {
                extra.push(PathBuf::from(loc));
            }
        }
        for l in &p.locations {
            if let Ok(Some(dir)) = resolve_location(l) {
                extra.push(dir);
            }
        }
        self.locator
            .locate(&p.executable, &extra)
            .ok_or_else(|| AppError::PrerequisiteMissing(format!("{} was not found. Install {} from the Tool Library first.", p.executable, m.name)))
    }

    fn audit(&self, kind: ActivityKind, tool: &ToolId, s: &str) {
        let _ = self.activity.record(ActivityEvent { at: now(), kind, tool_id: Some(tool.clone()), summary: redact(s) });
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
}

fn notes_for(p: &RunSpec, scope_present: bool) -> Vec<String> {
    let mut n = vec!["Runs with your normal Windows permissions. Vixalune never asks for administrator rights for a run.".to_string()];
    if scope_present {
        n.push("Scope checks cover the addresses a name resolves to now and plain-http redirects. They are a safety net, not proof that you are authorised to test the target.".into());
    }
    n.push(format!("The run stops by itself after {} seconds.", p.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS)));
    n
}

#[async_trait]
impl ToolRunner for NativeToolRunner {
    async fn preview(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &crate::domain::container::RunApprovals) -> AppResult<RunPreview> {
        let inner = &self.inner;
        let (m, p) = inner.resolve(id, profile)?;
        let preview_dir = inner.support.run_dir(OperationId::new());
        let built = inner.support.build(&p, values, &preview_dir)?;
        inner.support.check_paths(&built, inner.home.as_deref())?;
        let scope = inner.support.assess_scope(&built).await?;
        // A blocked target is an error already at preview time; "needs confirmation" is shown for the user to act on.
        if let Some(r) = &scope {
            if r.verdict == crate::domain::scope::Verdict::Blocked || r.redirects.iter().any(|h| crate::domain::scope::verdict_for(h.locality) != crate::domain::scope::Verdict::Allowed) {
                r.check(approvals.scope.as_ref())?;
            }
        }
        let mut notes = notes_for(&p, scope.is_some());
        if inner.locate(&m, &p).is_err() {
            notes.push(format!("{} is not installed yet; install it from the Tool Library before running.", m.name));
        }
        let working_dir = match p.working_dir {
            WorkingDir::RunDir => "A fresh empty folder for this run (the only place the tool may write result files).".to_string(),
            WorkingDir::InstallDir => "The tool's own install folder.".to_string(),
        };
        let mut command = built.preview;
        command.display = redact(&command.display);
        Ok(RunPreview {
            command,
            container: None,
            scope,
            network_activity: p.network_activity,
            timeout_secs: p.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
            output_format: p.output_format,
            expected_output: p.expected_output.clone(),
            working_dir,
            notes,
        })
    }

    async fn start(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &crate::domain::container::RunApprovals) -> AppResult<OperationId> {
        let inner = &self.inner;
        let (m, p) = inner.resolve(id, profile)?;
        let op = OperationId::new();
        let run_dir = inner.support.run_dir(op);

        // 1. typed parameters -> argument array; 2. real file system; 3. scope (fresh resolution) and confirmation.
        let built = inner.support.build(&p, values, &run_dir)?;
        inner.support.check_paths(&built, inner.home.as_deref())?;
        let scope = inner.support.enforce_scope(&built, approvals.scope.as_ref()).await?;
        let exe = inner.locate(&m, &p)?;

        let cancel = CancellationToken::new();
        let ts = now();
        let mut job = Job {
            id: op,
            tool_id: id.clone(),
            kind: JobKind::Run,
            phase: JobPhase::Queued,
            message: "Queued".into(),
            percent: None,
            started_at: ts.clone(),
            updated_at: ts.clone(),
            finished_at: None,
            exit_code: None,
            allow_elevation: false,
            error: None,
        };
        inner.store.create_job(&job)?; // Busy if the tool is installing or already running.
        if let Err(e) = std::fs::create_dir_all(&run_dir) {
            job.error = Some(JobError::from_app(&AppError::from(e), None));
            let _ = inner.step(&mut job, JobEvent::Fail, "The run folder could not be created");
            return Err(AppError::Io(format!("the run folder could not be created: {}", run_dir.display())));
        }
        let display = redact(&built.preview.display);
        let record = RunRecord {
            id: op,
            tool_id: id.clone(),
            tool_name: m.name.clone(),
            profile_id: p.id.clone(),
            profile_label: p.label.clone(),
            tool_version: inner.store.get_installation(id).ok().flatten().and_then(|r| r.version),
            started_at: ts,
            finished_at: None,
            duration_ms: None,
            exit_code: None,
            outcome: None,
            command: display.clone(),
            targets: scope.as_ref().map(|r| r.targets.iter().map(|t| t.canonical.clone()).collect()).unwrap_or_default(),
            scope: RunSupport::scope_summary(scope.as_ref(), p.network_activity),
            network_activity: p.network_activity,
            output_format: p.output_format,
            run_dir: Some(run_dir.display().to_string()),
            result_path: None,
        };
        inner.support.record_start(&record);
        inner.running.lock().unwrap().insert(op, cancel.clone());
        inner.audit(ActivityKind::RunStarted, id, &format!("{} '{}' scope: {}", m.name, p.id, record.scope));

        let cwd = match p.working_dir {
            WorkingDir::RunDir => run_dir.clone(),
            WorkingDir::InstallDir => exe.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| run_dir.clone()),
        };
        let spec = CommandSpec { program: exe, args: built.preview.args.clone(), timeout: Duration::from_secs(u64::from(p.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS))) };
        let opts = ExecOptions { cwd: Some(cwd), env: p.env.iter().map(|e| (e.name.clone(), e.value.clone())).collect() };

        let inner = self.inner.clone();
        tokio::spawn(async move {
            let sink = JobSink { store: inner.store.clone(), id: op };
            let started = inner.step(&mut job, JobEvent::StartChecks, "Starting").and_then(|_| inner.step(&mut job, JobEvent::BeginRun, &format!("Running {display}")));
            let result = match started {
                Ok(()) => {
                    let _ = inner.store.append_output(op, OutputStream::Stdout, &format!("> {display}"));
                    inner.executor.run_in(spec, opts, &sink, &cancel).await
                }
                Err(e) => Err(e),
            };
            let (exit, timed_out, cancelled, start_error) = match &result {
                Ok(r) => match r.outcome {
                    CommandOutcome::Exited(c) => (Some(c), false, false, None),
                    CommandOutcome::TimedOut => (None, true, false, None),
                    CommandOutcome::Cancelled => (None, false, true, None),
                },
                Err(e) => (None, false, false, Some(e.clone())),
            };
            let output = inner.store.job_output(op, 100_000).map(|l| l.join("\n")).unwrap_or_default();
            let outcome = match classify_outcome(exit, timed_out, cancelled, &p.exit_codes, &p.findings_patterns, &output) {
                Ok(o) => o,
                Err(_) => RunOutcome::Failed,
            };
            let tail = || output.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
            match outcome {
                RunOutcome::Clean | RunOutcome::Findings => {
                    job.exit_code = exit;
                    let msg = if outcome == RunOutcome::Findings { "Finished: the tool reported findings" } else { "Finished: nothing reported" };
                    let _ = inner.step(&mut job, JobEvent::RunDone, msg);
                    inner.audit(ActivityKind::RunFinished, &job.tool_id, &format!("{outcome:?}, exit code {}", exit.unwrap_or_default()));
                }
                RunOutcome::Cancelled => {
                    let _ = inner.step(&mut job, JobEvent::RequestCancel, "Cancelling");
                    let _ = inner.step(&mut job, JobEvent::CancelDone, "Cancelled; the process was stopped");
                    inner.audit(ActivityKind::RunTerminated, &job.tool_id, "cancelled by the user");
                }
                RunOutcome::TimedOut | RunOutcome::Failed => {
                    let e = match (&start_error, timed_out, exit) {
                        (Some(e), _, _) => e.clone(),
                        (_, true, _) => AppError::TimedOut(format!("the run exceeded {} seconds and was stopped", p.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS))),
                        (_, _, code) => AppError::CommandFailed { code, message: format!("the tool exited with code {}, which this profile does not treat as success", code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into())) },
                    };
                    job.exit_code = exit;
                    job.error = Some(JobError::from_app(&e, Some(tail())));
                    if inner.step(&mut job, JobEvent::Fail, &e.to_string()).is_err() {
                        job.phase = JobPhase::Failed;
                        job.finished_at = Some(now());
                        let _ = inner.store.update_job(&job);
                    }
                    inner.audit(ActivityKind::RunTerminated, &job.tool_id, e.code());
                }
            }
            inner.support.record_finish(op, exit, outcome);
            inner.running.lock().unwrap().remove(&op);
        });
        Ok(op)
    }
}

#[cfg(test)]
#[path = "native_runner_tests.rs"]
mod tests;
