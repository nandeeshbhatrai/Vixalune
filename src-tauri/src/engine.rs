//! The installation engine: owns job lifecycles, coordinates adapters, persists everything and
//! keeps an audit trail. It talks to the outside world only through ports, so it is fully
//! testable with mock adapters/executors.
//!
//! Lifecycle: `queued -> checking prerequisites -> [awaiting consent] -> installing -> verifying
//! -> succeeded`, with `failed`, `cancelling -> [rolling back] -> cancelled`, and `uninstalling`.
//! A job only reaches `succeeded` when the manifest's verification procedure passes.

use crate::domain::activity::{ActivityEvent, ActivityKind};
use crate::domain::failure::{FailureKind, FailureLayer, PlatformFailure};
use crate::domain::install_state::InstallState;
use crate::domain::jobs::{
    InstallationRecord, InstalledTool, Job, JobError, JobEvent, JobKind, JobPhase, ToolStateEntry, VerificationLevel, VerificationOutcome,
    VerificationResult,
};
use crate::domain::manifest::{ElevationNeed, InstallMethod, InstallSpec, ToolManifest};
use crate::domain::process::{CancellationToken, InstallPlan, OutputStream};
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::{
    ActivityLogger, AdapterContext, DependencyResolver, EnvironmentDetector, InstallationManager, InstallerAdapter, JobRegistry, JobStore,
    LineSink, PreflightReport, PresenceStatus, ToolQuery, ToolRegistry, VerifyReport,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

const OUTPUT_LIMIT: usize = 5000;
const JOB_LIST_LIMIT: usize = 200;

pub struct EngineDeps {
    pub registry: Arc<dyn ToolRegistry>,
    pub detector: Arc<dyn EnvironmentDetector>,
    pub resolver: Arc<dyn DependencyResolver>,
    pub adapters: Vec<Arc<dyn InstallerAdapter>>,
    pub store: Arc<dyn JobStore>,
    pub activity: Arc<dyn ActivityLogger>,
}

const GIB: u64 = 1024 * 1024 * 1024;

/// Free space required before a job starts: the manifest's own estimate with headroom, or a
/// conservative default per install method.
pub fn required_disk_bytes(m: &ToolManifest, spec: &InstallSpec) -> u64 {
    match m.storage.approx_disk_mb {
        Some(mb) => u64::from(mb) * 3 / 2 * 1024 * 1024 + GIB / 2,
        None => match spec {
            InstallSpec::Winget { .. } | InstallSpec::DownloadArchive { .. } => GIB,
            InstallSpec::WslApt { .. } => 2 * GIB,
            InstallSpec::ContainerImage { .. } => 5 * GIB,
        },
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

struct Handle {
    cancel: CancellationToken,
    consent: Arc<Notify>,
    approved: Arc<AtomicBool>,
}

struct Inner {
    deps: EngineDeps,
    running: Mutex<HashMap<OperationId, Handle>>,
    /// Result of the last presence check per tool: `true` = installed, `false` = absent.
    detection: Mutex<HashMap<ToolId, bool>>,
}

#[derive(Clone)]
pub struct InstallEngine {
    inner: Arc<Inner>,
}

impl InstallEngine {
    pub fn new(deps: EngineDeps) -> Self {
        Self { inner: Arc::new(Inner { deps, running: Mutex::new(HashMap::new()), detection: Mutex::new(HashMap::new()) }) }
    }

    /// Install methods with a working adapter in this build.
    pub fn supported_methods(&self) -> Vec<InstallMethod> {
        self.inner.deps.adapters.iter().map(|a| a.method()).collect()
    }

    /// Call once at startup. Jobs left active by a previous run were interrupted; they are closed
    /// out (and audited) so their tools are not stuck busy.
    pub fn recover(&self) -> AppResult<Vec<Job>> {
        let jobs = self.inner.deps.store.interrupt_active(&now())?;
        for j in &jobs {
            self.inner.audit(ActivityKind::InstallFailed, Some(&j.tool_id), &format!("{:?} job closed after restart ({})", j.kind, j.phase.as_str()));
        }
        Ok(jobs)
    }

    /// Wait until a job reaches a terminal phase. Intended for tests and tooling.
    pub async fn wait_for_terminal(&self, id: OperationId, timeout: Duration) -> AppResult<Job> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let job = self.inner.deps.store.get_job(id)?;
            if job.phase.is_terminal() {
                return Ok(job);
            }
            if tokio::time::Instant::now() > deadline {
                return Err(AppError::TimedOut(format!("job {id} still {}", job.phase.as_str())));
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    pub async fn wait_for_phase(&self, id: OperationId, phase: JobPhase, timeout: Duration) -> AppResult<Job> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let job = self.inner.deps.store.get_job(id)?;
            if job.phase == phase {
                return Ok(job);
            }
            if job.phase.is_terminal() || tokio::time::Instant::now() > deadline {
                return Err(AppError::TimedOut(format!("job {id} is {} (wanted {})", job.phase.as_str(), phase.as_str())));
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    async fn start_job(&self, tool: &ToolId, kind: JobKind, allow_elevation: bool) -> AppResult<OperationId> {
        let inner = &self.inner;
        let manifest = inner.deps.registry.get(tool)?.manifest;
        match kind {
            JobKind::Uninstall if !manifest.lifecycle.can_uninstall => {
                return Err(AppError::Unsupported(format!("{} cannot be uninstalled by Vixalune", manifest.name)))
            }
            JobKind::Update if !manifest.lifecycle.can_update => {
                return Err(AppError::Unsupported(format!("{} cannot be updated by Vixalune", manifest.name)))
            }
            _ => {}
        }
        if inner.adapter_for_any(&manifest).is_none() {
            return Err(AppError::Unsupported(format!(
                "no installer in this version of Vixalune supports {} (needs: {})",
                manifest.name,
                manifest.install.iter().map(|s| format!("{:?}", s.method())).collect::<Vec<_>>().join(" or ")
            )));
        }
        let ts = now();
        let job = Job {
            id: OperationId::new(),
            tool_id: tool.clone(),
            kind,
            phase: JobPhase::Queued,
            message: "Queued".into(),
            percent: None,
            started_at: ts.clone(),
            updated_at: ts,
            finished_at: None,
            exit_code: None,
            allow_elevation,
            error: None,
        };
        inner.deps.store.create_job(&job)?; // Busy if this tool already has an active job.

        let handle = Handle { cancel: CancellationToken::new(), consent: Arc::new(Notify::new()), approved: Arc::new(AtomicBool::new(false)) };
        let run = Run {
            inner: inner.clone(),
            job: Arc::new(Mutex::new(job.clone())),
            manifest,
            cancel: handle.cancel.clone(),
            consent: handle.consent.clone(),
            approved: handle.approved.clone(),
            was_installed_before: AtomicBool::new(false),
            touched: AtomicBool::new(false),
        };
        inner.running.lock().unwrap().insert(job.id, handle);
        inner.audit(ActivityKind::InstallStarted, Some(tool), &format!("{kind:?} requested"));
        let id = job.id;
        tokio::spawn(async move { run.execute().await });
        Ok(id)
    }
}

impl Inner {
    fn audit(&self, kind: ActivityKind, tool: Option<&ToolId>, summary: &str) {
        let ev = ActivityEvent { at: now(), kind, tool_id: tool.cloned(), summary: summary.to_string() };
        if let Err(e) = self.deps.activity.record(ev) {
            tracing::warn!("could not record audit event: {e}");
        }
    }

    fn adapter_for(&self, spec: &InstallSpec) -> Option<Arc<dyn InstallerAdapter>> {
        self.deps.adapters.iter().find(|a| a.supports(spec)).cloned()
    }

    fn adapter_for_any(&self, m: &ToolManifest) -> Option<(Arc<dyn InstallerAdapter>, InstallSpec)> {
        m.install.iter().find_map(|s| self.adapter_for(s).map(|a| (a, s.clone())))
    }

    fn set_detection(&self, tool: &ToolId, installed: bool) {
        self.detection.lock().unwrap().insert(tool.clone(), installed);
    }
}

// ------------------------------------------------------------------ one running job

struct Run {
    inner: Arc<Inner>,
    job: Arc<Mutex<Job>>,
    manifest: ToolManifest,
    cancel: CancellationToken,
    consent: Arc<Notify>,
    approved: Arc<AtomicBool>,
    was_installed_before: AtomicBool,
    /// Set once the job has begun changing the system (install/uninstall). Only then is the real state re-read afterwards.
    touched: AtomicBool,
}

struct Prepared {
    adapter: Arc<dyn InstallerAdapter>,
    spec: InstallSpec,
    plan: InstallPlan,
    preflight: Option<PreflightReport>,
}

/// Streams adapter output into the job's stored output and keeps progress current.
struct JobSink {
    job: Arc<Mutex<Job>>,
    store: Arc<dyn JobStore>,
}

fn parse_percent(line: &str) -> Option<u8> {
    let idx = line.rfind('%')?;
    let digits: String = line[..idx].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse::<u8>().ok().filter(|p| *p <= 100)
}

impl LineSink for JobSink {
    fn line(&self, stream: OutputStream, text: &str) {
        let id = self.job.lock().unwrap().id;
        if let Err(e) = self.store.append_output(id, stream, text) {
            tracing::warn!("could not store job output: {e}");
        }
        let snapshot = {
            let mut j = self.job.lock().unwrap();
            let pct = parse_percent(text);
            let changed = pct.is_some() && pct != j.percent;
            if let Some(p) = pct {
                j.percent = Some(p);
            }
            if pct.is_none() {
                j.message = text.chars().take(160).collect();
            }
            j.updated_at = now();
            (changed || pct.is_none()).then(|| j.clone())
        };
        if let Some(j) = snapshot {
            let _ = self.store.update_job(&j);
        }
    }
}

impl Run {
    fn snapshot(&self) -> Job {
        self.job.lock().unwrap().clone()
    }

    fn store(&self) -> &Arc<dyn JobStore> {
        &self.inner.deps.store
    }

    fn sink(&self) -> JobSink {
        JobSink { job: self.job.clone(), store: self.store().clone() }
    }

    fn transition(&self, event: JobEvent, message: &str) -> AppResult<()> {
        let snap = {
            let mut j = self.job.lock().unwrap();
            j.phase = j.phase.apply(event)?;
            if matches!(j.phase, JobPhase::Installing | JobPhase::Uninstalling) {
                self.touched.store(true, Ordering::SeqCst);
            }
            j.message = message.to_string();
            j.updated_at = now();
            if j.phase.is_terminal() {
                j.finished_at = Some(j.updated_at.clone());
            }
            j.clone()
        };
        self.store().update_job(&snap)
    }

    fn set_error(&self, error: JobError, exit_code: Option<i32>) {
        let snap = {
            let mut j = self.job.lock().unwrap();
            j.error = Some(error);
            j.exit_code = exit_code.or(j.exit_code);
            j.updated_at = now();
            j.clone()
        };
        let _ = self.store().update_job(&snap);
    }

    fn check_cancelled(&self) -> AppResult<()> {
        if self.cancel.is_cancelled() {
            Err(AppError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn tool_id(&self) -> &ToolId {
        &self.manifest.id
    }

    async fn execute(self) {
        let id = self.snapshot().id;
        let result = self.drive().await;
        match result {
            Ok(()) => {}
            Err(AppError::Cancelled) => self.handle_cancel().await,
            Err(e) => self.handle_failure(e).await,
        }
        self.finalize().await;
        self.inner.running.lock().unwrap().remove(&id);
    }

    async fn drive(&self) -> AppResult<()> {
        self.transition(JobEvent::StartChecks, "Checking prerequisites")?;
        match self.snapshot().kind {
            JobKind::Install | JobKind::Update => self.install_flow().await,
            JobKind::Uninstall => self.uninstall_flow().await,
            JobKind::Verify => self.verify_flow().await,
            JobKind::Run | JobKind::Setup => Err(AppError::NotImplemented("this job kind is handled by its own manager")),
        }
    }

    // -------------------------------------------------------------- preparation

    async fn prepare(&self, with_preflight: bool) -> AppResult<Prepared> {
        self.check_cancelled()?;
        let env = self.inner.deps.detector.detect().await;
        let inner = self.inner.clone();
        let plan = self.inner.deps.resolver.resolve(&self.manifest, &env, &move |s| inner.adapter_for(s).is_some())?;
        if let Some(reason) = &plan.unsupported_reason {
            return Err(AppError::Unsupported(reason.clone()));
        }
        if !plan.missing_prerequisites.is_empty() {
            let detail = plan
                .prerequisites
                .iter()
                .filter(|p| p.satisfied != Some(true))
                .map(|p| format!("{:?}: {}", p.spec.kind, p.spec.reason))
                .collect::<Vec<_>>()
                .join(" | ");
            return Err(AppError::PrerequisiteMissing(detail));
        }
        if let Some(free) = env.free_disk_bytes {
            let need = required_disk_bytes(&self.manifest, &plan.method);
            if free < need {
                return Err(AppError::Platform(PlatformFailure::new(
                    FailureKind::InsufficientDisk,
                    FailureLayer::Windows,
                    format!("only {:.1} GB free; about {:.1} GB is needed", free as f64 / GIB as f64, need as f64 / GIB as f64),
                )));
            }
        }
        for dep in &plan.dependencies {
            let ok = matches!(self.store().get_installation(dep)?, Some(r) if r.state == InstallState::Installed);
            if !ok {
                return Err(AppError::PrerequisiteMissing(format!("install the tool '{dep}' first; {} depends on it", self.manifest.name)));
            }
        }
        let adapter = self
            .inner
            .adapter_for(&plan.method)
            .ok_or_else(|| AppError::Unsupported("no adapter for the selected install method".into()))?;
        let spec = plan.method.clone();
        let preflight = if with_preflight {
            let sink = self.sink();
            let ctx = AdapterContext { tool: &self.manifest, cancel: &self.cancel, sink: &sink };
            Some(adapter.preflight(&ctx, &spec).await?)
        } else {
            None
        };
        self.check_cancelled()?;
        Ok(Prepared { adapter, spec, plan, preflight })
    }

    async fn wait_for_consent(&self) -> AppResult<()> {
        loop {
            self.check_cancelled()?;
            if self.approved.load(Ordering::SeqCst) {
                return Ok(());
            }
            tokio::select! {
                _ = self.consent.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }
    }

    async fn maybe_consent(&self, plan: &InstallPlan) -> AppResult<()> {
        if plan.elevation != ElevationNeed::None && !self.snapshot().allow_elevation {
            self.transition(JobEvent::RequireConsent, "Waiting for your approval (administrator rights may be requested)")?;
            self.wait_for_consent().await?;
        }
        Ok(())
    }

    // -------------------------------------------------------------- flows

    async fn install_flow(&self) -> AppResult<()> {
        let kind = self.snapshot().kind;
        let p = self.prepare(true).await?;
        let pre = p.preflight.clone().unwrap_or_default();
        self.was_installed_before.store(pre.already_installed, Ordering::SeqCst);
        let sink = self.sink();
        let ctx = AdapterContext { tool: &self.manifest, cancel: &self.cancel, sink: &sink };

        if kind == JobKind::Update && !pre.already_installed {
            return Err(AppError::Validation(format!("{} is not installed, so it cannot be updated", self.manifest.name)));
        }
        if kind == JobKind::Install && pre.already_installed {
            // Adopt an existing installation: verify it rather than reinstalling over it.
            self.transition(JobEvent::BeginVerify, "Already installed; verifying")?;
        } else {
            self.maybe_consent(&p.plan).await?;
            let verb = if kind == JobKind::Update { "Updating" } else { "Installing" };
            self.transition(JobEvent::BeginInstall, &format!("{verb} {}", self.manifest.name))?;
            let out = p.adapter.install(&ctx, &p.spec, kind == JobKind::Update).await?;
            if let Some(code) = out.exit_code {
                self.set_exit_code(code);
            }
            self.check_cancelled()?;
            self.transition(JobEvent::InstallDone, "Verifying installation")?;
        }
        self.verify_and_record(&*p.adapter, &p.spec, &ctx, p.method()).await
    }

    fn set_exit_code(&self, code: i32) {
        let snap = {
            let mut j = self.job.lock().unwrap();
            j.exit_code = Some(code);
            j.clone()
        };
        let _ = self.store().update_job(&snap);
    }

    async fn verify_flow(&self) -> AppResult<()> {
        let p = self.prepare(false).await?;
        let sink = self.sink();
        let ctx = AdapterContext { tool: &self.manifest, cancel: &self.cancel, sink: &sink };
        self.transition(JobEvent::BeginVerify, "Verifying")?;
        self.verify_and_record(&*p.adapter, &p.spec, &ctx, p.method()).await
    }

    async fn verify_and_record(&self, adapter: &dyn InstallerAdapter, spec: &InstallSpec, ctx: &AdapterContext<'_>, method: InstallMethod) -> AppResult<()> {
        let report: VerifyReport = adapter.verify(ctx, spec).await?;
        self.check_cancelled()?;
        let verification = VerificationResult {
            at: now(),
            outcome: report.outcome,
            level: report.level,
            detail: report.detail.clone(),
        };
        match report.outcome {
            VerificationOutcome::Passed => {
                let version = match adapter.status(ctx, spec).await {
                    Ok(PresenceStatus::Installed { version }) => version,
                    _ => None,
                };
                self.put_record(InstallState::Installed, method, version, report.location.clone(), Some(verification));
                self.inner.set_detection(self.tool_id(), true);
                self.transition(JobEvent::VerifyPassed, &format!("Installed and verified. {}", report.detail))?;
                self.inner.audit(ActivityKind::InstallSucceeded, Some(self.tool_id()), &format!("{} verified ({:?})", self.manifest.name, report.level));
                self.inner.audit(ActivityKind::Verified, Some(self.tool_id()), &report.detail);
            }
            outcome => {
                let present = matches!(adapter.status(ctx, spec).await, Ok(PresenceStatus::Installed { .. }));
                if present {
                    self.put_record(InstallState::VerificationFailed, method, None, report.location.clone(), Some(verification));
                    self.inner.set_detection(self.tool_id(), true);
                } else {
                    let _ = self.store().delete_installation(self.tool_id());
                    self.inner.set_detection(self.tool_id(), false);
                }
                let (code, message) = if outcome == VerificationOutcome::Unavailable {
                    ("verification_unavailable", "Verification could not be performed, so the install is not reported as successful.")
                } else {
                    ("verification_failed", "The tool did not pass verification, so the install is not reported as successful.")
                };
                self.set_error(
                    JobError {
                        code: code.into(),
                        message: message.into(),
                        recovery: AppError::VerificationFailed(String::new()).recovery().into(),
                        detail: Some(report.detail.clone()),
                    },
                    None,
                );
                self.transition(JobEvent::VerifyFailed, &report.detail)?;
                self.inner.audit(ActivityKind::InstallFailed, Some(self.tool_id()), &format!("verification {outcome:?}: {}", report.detail));
            }
        }
        Ok(())
    }

    async fn uninstall_flow(&self) -> AppResult<()> {
        let p = self.prepare(false).await?;
        let sink = self.sink();
        let ctx = AdapterContext { tool: &self.manifest, cancel: &self.cancel, sink: &sink };
        match p.adapter.status(&ctx, &p.spec).await? {
            PresenceStatus::NotInstalled => {
                let _ = self.store().delete_installation(self.tool_id());
                self.inner.set_detection(self.tool_id(), false);
                self.transition(JobEvent::BeginUninstall, "Not installed; nothing to remove")?;
                self.transition(JobEvent::UninstallDone, "Not installed; nothing to remove")?;
                return Ok(());
            }
            PresenceStatus::Unknown(why) => return Err(AppError::Environment(format!("cannot tell whether the tool is installed: {why}"))),
            PresenceStatus::Installed { .. } => {}
        }
        self.maybe_consent(&p.plan).await?;
        self.transition(JobEvent::BeginUninstall, &format!("Uninstalling {}", self.manifest.name))?;
        let out = p.adapter.uninstall(&ctx, &p.spec).await?;
        if let Some(code) = out.exit_code {
            self.set_exit_code(code);
        }
        self.check_cancelled()?;
        // Do not claim success unless the package manager agrees it is gone.
        match p.adapter.status(&ctx, &p.spec).await? {
            PresenceStatus::NotInstalled => {}
            other => {
                return Err(AppError::VerificationFailed(format!("after uninstalling, the package manager still reports: {other:?}")));
            }
        }
        let _ = self.store().delete_installation(self.tool_id());
        self.inner.set_detection(self.tool_id(), false);
        self.transition(JobEvent::UninstallDone, "Uninstalled")?;
        self.inner.audit(ActivityKind::Uninstalled, Some(self.tool_id()), &self.manifest.name);
        Ok(())
    }

    // -------------------------------------------------------------- failure handling

    fn put_record(&self, state: InstallState, adapter: InstallMethod, version: Option<String>, location: Option<String>, verification: Option<VerificationResult>) {
        let tool = self.tool_id().clone();
        let previous = self.store().get_installation(&tool).ok().flatten();
        let ts = now();
        let rec = InstallationRecord {
            tool_id: tool,
            state,
            version: version.or_else(|| previous.as_ref().and_then(|p| p.version.clone())),
            location: location.or_else(|| previous.as_ref().and_then(|p| p.location.clone())),
            adapter,
            manifest_version: self.manifest.manifest_version.clone(),
            installed_at: previous.and_then(|p| p.installed_at).or_else(|| Some(ts.clone())),
            updated_at: ts,
            last_verification: verification,
        };
        if let Err(e) = self.store().put_installation(&rec) {
            tracing::error!("could not persist installation record: {e}");
        }
    }

    /// Remove what this job added. Runs with a fresh token because the job's own is cancelled.
    /// Only done when the tool was *not* installed before this job (so we never remove something
    /// the user already had).
    async fn rollback(&self) {
        if self.snapshot().kind != JobKind::Install || self.was_installed_before.load(Ordering::SeqCst) {
            return;
        }
        let Some((adapter, spec)) = self.inner.adapter_for_any(&self.manifest) else { return };
        let fresh = CancellationToken::new();
        let sink = self.sink();
        let ctx = AdapterContext { tool: &self.manifest, cancel: &fresh, sink: &sink };
        match adapter.status(&ctx, &spec).await {
            Ok(PresenceStatus::Installed { .. }) => {
                sink.line(OutputStream::Stdout, "Rolling back the partial installation");
                match adapter.uninstall(&ctx, &spec).await {
                    Ok(_) => {
                        let _ = self.store().delete_installation(self.tool_id());
                        self.inner.set_detection(self.tool_id(), false);
                    }
                    Err(e) => sink.line(OutputStream::Stderr, &format!("Rollback failed: {e}. Use Uninstall to remove it manually.")),
                }
            }
            Ok(PresenceStatus::NotInstalled) => self.inner.set_detection(self.tool_id(), false),
            _ => {}
        }
    }

    async fn handle_failure(&self, e: AppError) {
        let snap = self.snapshot();
        let tail = self.store().job_output(snap.id, 8).unwrap_or_default().join(" | ");
        let code = match &e {
            AppError::CommandFailed { code, .. } => *code,
            _ => None,
        };
        let detail = [code.map(|c| format!("exit code {c} (0x{:08X})", c as u32)), (!tail.is_empty()).then(|| format!("last output: {tail}"))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; ");
        self.set_error(JobError::from_app(&e, (!detail.is_empty()).then_some(detail)), code);
        let was_installing = snap.phase == JobPhase::Installing;
        let message = e.to_string();
        if was_installing && snap.kind == JobKind::Install && !self.was_installed_before.load(Ordering::SeqCst) {
            if self.transition(JobEvent::BeginRollback, "Install failed; rolling back").is_ok() {
                self.rollback().await;
            }
        }
        if self.transition(JobEvent::Fail, &message).is_err() {
            self.force_phase(JobPhase::Failed, &message);
        }
        self.inner.audit(ActivityKind::InstallFailed, Some(self.tool_id()), &format!("{:?} failed: {}", snap.kind, e.code()));
    }

    async fn handle_cancel(&self) {
        let snap = self.snapshot();
        if self.transition(JobEvent::RequestCancel, "Cancelling").is_err() {
            self.force_phase(JobPhase::Cancelled, "Cancelled");
            return;
        }
        match snap.phase {
            JobPhase::Installing => {
                if snap.kind == JobKind::Install && !self.was_installed_before.load(Ordering::SeqCst) && self.transition(JobEvent::BeginRollback, "Rolling back").is_ok() {
                    self.rollback().await;
                }
            }
            JobPhase::Verifying => {
                // Installed but never verified: say so rather than claiming success.
                let present = self.inner.adapter_for_any(&self.manifest).map(|(_, s)| s.method());
                if let Some(method) = present {
                    self.put_record(
                        InstallState::VerificationFailed,
                        method,
                        None,
                        None,
                        Some(VerificationResult { at: now(), outcome: VerificationOutcome::Unavailable, level: VerificationLevel::None, detail: "Verification was cancelled".into() }),
                    );
                }
            }
            _ => {}
        }
        if self.transition(JobEvent::CancelDone, "Cancelled").is_err() {
            self.force_phase(JobPhase::Cancelled, "Cancelled");
        }
        self.inner.audit(ActivityKind::InstallFailed, Some(self.tool_id()), &format!("{:?} cancelled", snap.kind));
    }

    fn force_phase(&self, phase: JobPhase, message: &str) {
        let snap = {
            let mut j = self.job.lock().unwrap();
            j.phase = phase;
            j.message = message.to_string();
            j.updated_at = now();
            j.finished_at = Some(j.updated_at.clone());
            j.clone()
        };
        let _ = self.store().update_job(&snap);
    }

    /// After a job that did not end in a verified success, re-read the real state so the UI
    /// never shows a guess.
    async fn finalize(&self) {
        let snap = self.snapshot();
        if snap.phase == JobPhase::Succeeded || !self.touched.load(Ordering::SeqCst) {
            return;
        }
        let Some((adapter, spec)) = self.inner.adapter_for_any(&self.manifest) else { return };
        let fresh = CancellationToken::new();
        let sink = self.sink();
        let ctx = AdapterContext { tool: &self.manifest, cancel: &fresh, sink: &sink };
        match adapter.status(&ctx, &spec).await {
            Ok(PresenceStatus::NotInstalled) => {
                let _ = self.store().delete_installation(self.tool_id());
                self.inner.set_detection(self.tool_id(), false);
            }
            Ok(PresenceStatus::Installed { .. }) => self.inner.set_detection(self.tool_id(), true),
            _ => {}
        }
    }
}

impl Prepared {
    fn method(&self) -> InstallMethod {
        self.spec.method()
    }
}

// ------------------------------------------------------------------ trait implementations

fn phase_to_state(phase: JobPhase, kind: JobKind) -> InstallState {
    match phase {
        JobPhase::Queued | JobPhase::CheckingPrerequisites | JobPhase::AwaitingConsent => InstallState::Queued,
        JobPhase::Installing => InstallState::Installing,
        JobPhase::Verifying => InstallState::Verifying,
        JobPhase::Uninstalling => InstallState::Uninstalling,
        // A running tool is installed; the active job stops other operations on it.
        JobPhase::Running => InstallState::Installed,
        JobPhase::Cancelling | JobPhase::RollingBack => InstallState::Cancelling,
        JobPhase::Succeeded | JobPhase::Failed | JobPhase::Cancelled => {
            let _ = kind;
            InstallState::NotInstalled
        }
    }
}

impl InstallEngine {
    fn derive_state(&self, tool: &ToolId, jobs: &[Job]) -> Option<InstallState> {
        let inner = &self.inner;
        if let Some(active) = jobs.iter().find(|j| &j.tool_id == tool && j.phase.is_active()) {
            return Some(phase_to_state(active.phase, active.kind));
        }
        if let Ok(Some(rec)) = inner.deps.store.get_installation(tool) {
            return Some(rec.state);
        }
        let last = jobs.iter().find(|j| &j.tool_id == tool);
        if let Some(j) = last {
            if j.phase == JobPhase::Failed && j.kind != JobKind::Verify && j.kind != JobKind::Uninstall {
                return Some(InstallState::Failed);
            }
        }
        match inner.detection.lock().unwrap().get(tool) {
            Some(false) => Some(InstallState::NotInstalled),
            Some(true) => Some(InstallState::Installed),
            None => None,
        }
    }
}

#[async_trait]
impl InstallationManager for InstallEngine {
    fn state(&self, id: &ToolId) -> AppResult<InstallState> {
        let jobs = self.inner.deps.store.list_jobs(JOB_LIST_LIMIT)?;
        self.derive_state(id, &jobs).ok_or_else(|| AppError::Environment(format!("the state of '{id}' has not been determined yet")))
    }

    fn states(&self) -> AppResult<Vec<ToolStateEntry>> {
        let jobs = self.inner.deps.store.list_jobs(JOB_LIST_LIMIT)?;
        Ok(self
            .inner
            .deps
            .registry
            .list(&ToolQuery::default())
            .into_iter()
            .map(|e| ToolStateEntry { state: self.derive_state(&e.manifest.id, &jobs), tool_id: e.manifest.id })
            .collect())
    }

    fn installed(&self) -> AppResult<Vec<InstalledTool>> {
        let mut out = Vec::new();
        for rec in self.inner.deps.store.list_installations()? {
            let Ok(entry) = self.inner.deps.registry.get(&rec.tool_id) else { continue };
            let jobs = self.inner.deps.store.list_jobs(JOB_LIST_LIMIT)?;
            let state = self.derive_state(&rec.tool_id, &jobs).unwrap_or(rec.state);
            let runtime = entry.manifest.install.iter().find(|s| s.method() == rec.adapter).map(|s| s.runtime()).unwrap_or(entry.manifest.platforms[0]);
            out.push(InstalledTool {
                tool_id: rec.tool_id,
                version: rec.version,
                location: rec.location,
                runtime,
                state,
                last_verification: rec.last_verification,
            });
        }
        Ok(out)
    }

    async fn refresh_states(&self) -> AppResult<()> {
        let jobs = self.inner.deps.store.list_jobs(JOB_LIST_LIMIT)?;
        let fresh = CancellationToken::new();
        let sink = NullSink;
        for entry in self.inner.deps.registry.list(&ToolQuery::default()) {
            let m = entry.manifest;
            if jobs.iter().any(|j| j.tool_id == m.id && j.phase.is_active()) {
                continue;
            }
            let Some((adapter, spec)) = self.inner.adapter_for_any(&m) else { continue };
            let ctx = AdapterContext { tool: &m, cancel: &fresh, sink: &sink };
            match adapter.status(&ctx, &spec).await {
                Ok(PresenceStatus::Installed { version }) => {
                    self.inner.set_detection(&m.id, true);
                    if self.inner.deps.store.get_installation(&m.id)?.is_none() {
                        let ts = now();
                        self.inner.deps.store.put_installation(&InstallationRecord {
                            tool_id: m.id.clone(),
                            state: InstallState::Installed,
                            version,
                            location: None,
                            adapter: spec.method(),
                            manifest_version: m.manifest_version.clone(),
                            installed_at: None,
                            updated_at: ts,
                            last_verification: None,
                        })?;
                    }
                }
                Ok(PresenceStatus::NotInstalled) => {
                    self.inner.set_detection(&m.id, false);
                    self.inner.deps.store.delete_installation(&m.id)?;
                }
                Ok(PresenceStatus::Unknown(why)) => tracing::warn!("could not determine state of {}: {why}", m.id),
                Err(e) => tracing::warn!("state check for {} failed: {e}", m.id),
            }
        }
        Ok(())
    }

    async fn plan(&self, id: &ToolId) -> AppResult<InstallPlan> {
        let manifest = self.inner.deps.registry.get(id)?.manifest;
        let env = self.inner.deps.detector.detect().await;
        let inner = self.inner.clone();
        self.inner.deps.resolver.resolve(&manifest, &env, &move |s| inner.adapter_for(s).is_some())
    }

    async fn install(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId> {
        self.start_job(id, JobKind::Install, allow_elevation).await
    }

    async fn approve(&self, op: OperationId) -> AppResult<()> {
        let job = self.inner.deps.store.get_job(op)?;
        if job.phase != JobPhase::AwaitingConsent {
            return Err(AppError::InvalidTransition { from: job.phase.as_str().into(), event: "approve".into() });
        }
        let running = self.inner.running.lock().unwrap();
        let h = running.get(&op).ok_or_else(|| AppError::NotFound(format!("running job '{op}'")))?;
        h.approved.store(true, Ordering::SeqCst);
        h.consent.notify_one();
        Ok(())
    }

    async fn cancel(&self, op: OperationId) -> AppResult<()> {
        let job = self.inner.deps.store.get_job(op)?;
        if job.phase.is_terminal() || job.phase == JobPhase::Cancelling || job.phase == JobPhase::RollingBack {
            return Err(AppError::InvalidTransition { from: job.phase.as_str().into(), event: "cancel".into() });
        }
        let running = self.inner.running.lock().unwrap();
        let h = running.get(&op).ok_or_else(|| AppError::NotFound(format!("running job '{op}' (it belongs to a previous session)")))?;
        h.cancel.cancel();
        h.consent.notify_one();
        Ok(())
    }

    async fn retry(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId> {
        let jobs = self.inner.deps.store.list_jobs(JOB_LIST_LIMIT)?;
        match jobs.iter().find(|j| &j.tool_id == id) {
            Some(j) if matches!(j.phase, JobPhase::Failed | JobPhase::Cancelled) => self.start_job(id, j.kind, allow_elevation).await,
            Some(j) if j.phase.is_active() => Err(AppError::Busy(format!("'{id}' already has an operation in progress"))),
            _ => Err(AppError::Validation(format!("there is no failed or cancelled operation on '{id}' to retry"))),
        }
    }

    async fn update(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId> {
        self.start_job(id, JobKind::Update, allow_elevation).await
    }

    async fn uninstall(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId> {
        self.start_job(id, JobKind::Uninstall, allow_elevation).await
    }

    async fn verify(&self, id: &ToolId) -> AppResult<OperationId> {
        self.start_job(id, JobKind::Verify, false).await
    }
}

#[async_trait]
impl JobRegistry for InstallEngine {
    fn list(&self) -> AppResult<Vec<Job>> {
        self.inner.deps.store.list_jobs(JOB_LIST_LIMIT)
    }
    fn output(&self, id: OperationId) -> AppResult<Vec<String>> {
        self.inner.deps.store.get_job(id)?; // NotFound for unknown ids
        self.inner.deps.store.job_output(id, OUTPUT_LIMIT)
    }
    async fn cancel(&self, id: OperationId) -> AppResult<()> {
        InstallationManager::cancel(self, id).await
    }
}

struct NullSink;
impl LineSink for NullSink {
    fn line(&self, _: OutputStream, _: &str) {}
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;




/// Job registry that routes cancellation to whichever manager owns the job (installs/verifies,
/// container runs). Listing and output come from the shared store.
pub struct RoutedJobs {
    pub engine: InstallEngine,
    pub runner: Arc<crate::adapters::container_runner::ContainerToolRunner>,
}

#[async_trait]
impl JobRegistry for RoutedJobs {
    fn list(&self) -> AppResult<Vec<Job>> {
        JobRegistry::list(&self.engine)
    }
    fn output(&self, id: OperationId) -> AppResult<Vec<String>> {
        JobRegistry::output(&self.engine, id)
    }
    async fn cancel(&self, id: OperationId) -> AppResult<()> {
        match InstallationManager::cancel(&self.engine, id).await {
            Err(AppError::NotFound(_)) => self.runner.cancel(id).await,
            other => other,
        }
    }
}

