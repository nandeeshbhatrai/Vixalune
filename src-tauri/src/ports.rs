//! Application-layer interfaces ("ports"). The UI/command layer depends only on these traits;
//! platform adapters and storage implement them. Everything is `Send + Sync` so
//! implementations can be shared as `Arc<dyn Trait>` (constructor injection).

use crate::domain::activity::ActivityEvent;
use crate::domain::environment::EnvironmentReport;
use crate::domain::install_state::InstallState;
use crate::domain::jobs::{
    InstallationRecord, InstalledTool, Job, ToolStateEntry, VerificationLevel, VerificationOutcome,
};
use crate::domain::lab::{LabDefinition, LabState};
use crate::domain::manifest::{CommandPreview, InstallMethod, InstallSpec, ParamValue, ToolManifest};
use crate::domain::process::{CancellationToken, InstallPlan, OutputSink, OutputStream, ProcessSpec, ProcessStatus};
use crate::domain::{AppResult, LabId, OperationId, ProcessId, RunId, ToolId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// ------------------------------------------------------------------ catalog

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Shipped inside the application bundle.
    Bundled,
    /// Fetched from a remote source and validated, but not signed.
    RemoteUnsigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub manifest: ToolManifest,
    pub trust: TrustLevel,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolQuery {
    pub text: Option<String>,
    pub category: Option<crate::domain::manifest::Category>,
}

/// Read access to validated tool definitions.
pub trait ToolRegistry: Send + Sync {
    fn list(&self, query: &ToolQuery) -> Vec<CatalogEntry>;
    fn get(&self, id: &ToolId) -> AppResult<CatalogEntry>;
}

// ------------------------------------------------------------------ environment

pub trait EnvironmentDetector: Send + Sync {
    /// Blocking: probes external programs. Call from a blocking-capable thread.
    fn detect(&self) -> EnvironmentReport;
}

pub trait DependencyResolver: Send + Sync {
    /// Choose an install method for which `usable` is true and describe what the install needs.
    fn resolve(
        &self,
        manifest: &ToolManifest,
        env: &EnvironmentReport,
        usable: &dyn Fn(&InstallSpec) -> bool,
    ) -> AppResult<InstallPlan>;
}

// ------------------------------------------------------------------ command execution

/// A fixed program and argument array. There is no shell: `program` is an absolute path and each
/// element of `args` reaches the process as a single argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    Exited(i32),
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub outcome: CommandOutcome,
    /// Last lines of combined output, for diagnostics.
    pub tail: Vec<String>,
}

/// Receives output lines as they are produced. Implementations redact before persisting.
pub trait LineSink: Send + Sync {
    fn line(&self, stream: OutputStream, text: &str);
}

#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Runs to completion, streaming lines to `sink`. Kills the process tree on timeout or when
    /// `cancel` is triggered. Returns `Err` only if the process could not be started.
    async fn run(&self, spec: CommandSpec, sink: &dyn LineSink, cancel: &CancellationToken) -> AppResult<CommandResult>;
}

/// Finds executables using a freshly read system PATH plus explicit extra folders.
pub trait ExecutableLocator: Send + Sync {
    fn locate(&self, file_name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf>;
}

// ------------------------------------------------------------------ installation adapters

pub struct AdapterContext<'a> {
    pub tool: &'a ToolManifest,
    pub cancel: &'a CancellationToken,
    pub sink: &'a dyn LineSink,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreflightReport {
    pub already_installed: bool,
    pub installed_version: Option<String>,
    pub available_version: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceStatus {
    Installed { version: Option<String> },
    NotInstalled,
    /// Could not be determined (package manager missing, timeout, unexpected output).
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub outcome: VerificationOutcome,
    pub level: VerificationLevel,
    pub detail: String,
    /// Folder of the executable that was run, when one was found.
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub detail: String,
    pub exit_code: Option<i32>,
}

/// Executes one kind of [`InstallSpec`] using structured executables and argument arrays.
/// Every adapter supports: preflight, install, status, verify, uninstall (cleanup), and honours
/// `ctx.cancel` where the underlying tool allows.
#[async_trait]
pub trait InstallerAdapter: Send + Sync {
    fn method(&self) -> InstallMethod;
    fn supports(&self, spec: &InstallSpec) -> bool;
    /// Read-only checks (is the package manager usable, does the package exist, already installed?).
    async fn preflight(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PreflightReport>;
    /// `update` upgrades an existing installation instead of installing.
    async fn install(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec, update: bool) -> AppResult<InstallOutcome>;
    async fn status(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PresenceStatus>;
    /// Runs the manifest's verification procedure. `Unavailable` is never success.
    async fn verify(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<VerifyReport>;
    /// Documented cleanup: removes what `install` added, nothing else.
    async fn uninstall(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<InstallOutcome>;
}

// ------------------------------------------------------------------ persistence

/// Durable job and installation state (SQLite in production).
pub trait JobStore: Send + Sync {
    /// Fails with `AppError::Busy` if another job for the same tool is still active.
    fn create_job(&self, job: &Job) -> AppResult<()>;
    fn update_job(&self, job: &Job) -> AppResult<()>;
    fn get_job(&self, id: OperationId) -> AppResult<Job>;
    fn list_jobs(&self, limit: usize) -> AppResult<Vec<Job>>;
    fn active_job_for(&self, tool: &ToolId) -> AppResult<Option<Job>>;
    fn append_output(&self, id: OperationId, stream: OutputStream, text: &str) -> AppResult<()>;
    fn job_output(&self, id: OperationId, limit: usize) -> AppResult<Vec<String>>;
    /// After a restart: every job still marked active was interrupted. Marks them failed and returns them.
    fn interrupt_active(&self, now: &str) -> AppResult<Vec<Job>>;
    fn put_installation(&self, record: &InstallationRecord) -> AppResult<()>;
    fn get_installation(&self, tool: &ToolId) -> AppResult<Option<InstallationRecord>>;
    fn list_installations(&self) -> AppResult<Vec<InstallationRecord>>;
    fn delete_installation(&self, tool: &ToolId) -> AppResult<()>;
}

/// Owns installation lifecycle jobs. Elevation is never inferred: the caller passes
/// `allow_elevation`, and if the tool may need it and it was not given, the job waits in
/// `AwaitingConsent` until `approve` is called.
#[async_trait]
pub trait InstallationManager: Send + Sync {
    fn state(&self, id: &ToolId) -> AppResult<InstallState>;
    /// Known states for catalog tools. Tools whose state is not yet known have `state: None`.
    fn states(&self) -> AppResult<Vec<ToolStateEntry>>;
    fn installed(&self) -> AppResult<Vec<InstalledTool>>;
    /// Ask each adapter whether tools are present and persist the answers.
    async fn refresh_states(&self) -> AppResult<()>;
    async fn plan(&self, id: &ToolId) -> AppResult<InstallPlan>;
    async fn install(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn approve(&self, op: OperationId) -> AppResult<()>;
    async fn cancel(&self, op: OperationId) -> AppResult<()>;
    async fn retry(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn update(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn uninstall(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn verify(&self, id: &ToolId) -> AppResult<OperationId>;
}

// ------------------------------------------------------------------ running

#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, spec: ProcessSpec, output: Arc<dyn OutputSink>) -> AppResult<ProcessId>;
    /// Terminates the process and its children.
    async fn terminate(&self, id: ProcessId) -> AppResult<()>;
    fn status(&self, id: ProcessId) -> AppResult<ProcessStatus>;
}

#[async_trait]
pub trait ToolRunner: Send + Sync {
    /// Validate parameters and build the exact command without running it.
    fn preview(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>) -> AppResult<CommandPreview>;
    async fn start(
        &self,
        id: &ToolId,
        profile: &str,
        values: &HashMap<String, ParamValue>,
        output: Arc<dyn OutputSink>,
    ) -> AppResult<RunId>;
    async fn stop(&self, run: RunId) -> AppResult<()>;
}

// ------------------------------------------------------------------ labs

#[async_trait]
pub trait LabManager: Send + Sync {
    fn list(&self) -> Vec<LabDefinition>;
    fn state(&self, id: &LabId) -> AppResult<LabState>;
    async fn start(&self, id: &LabId) -> AppResult<()>;
    async fn stop(&self, id: &LabId) -> AppResult<()>;
}

// ------------------------------------------------------------------ jobs and links

/// Read access to jobs for the Running Jobs screen.
#[async_trait]
pub trait JobRegistry: Send + Sync {
    fn list(&self) -> AppResult<Vec<Job>>;
    fn output(&self, id: OperationId) -> AppResult<Vec<String>>;
    async fn cancel(&self, id: OperationId) -> AppResult<()>;
}

/// Opens a catalog link in the user's default browser. The caller selects a link by kind;
/// the URL always comes from a validated manifest, never from the UI.
pub trait LinkOpener: Send + Sync {
    fn open(&self, url: &str) -> AppResult<()>;
}

// ------------------------------------------------------------------ activity

pub trait ActivityLogger: Send + Sync {
    /// Implementations must redact secrets before persisting.
    fn record(&self, event: ActivityEvent) -> AppResult<()>;
    fn recent(&self, limit: usize) -> AppResult<Vec<ActivityEvent>>;
}
