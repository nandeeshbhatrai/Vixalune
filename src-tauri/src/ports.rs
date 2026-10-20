//! Application-layer interfaces ("ports"). The UI/command layer depends only on these traits;
//! platform adapters and storage implement them. Everything is `Send + Sync` so
//! implementations can be shared as `Arc<dyn Trait>` (constructor injection).

use crate::domain::activity::ActivityEvent;
use crate::domain::environment::EnvironmentReport;
use crate::domain::install_state::InstallState;
use crate::domain::jobs::{
    InstallationRecord, InstalledTool, Job, ToolStateEntry, VerificationLevel, VerificationOutcome,
};
use crate::domain::container::{RunApprovals, RunSummary};
use crate::domain::lab::{LabDefinition, LabState};
use crate::domain::platform::{PlatformReport, SetupActionId};
use crate::domain::runs::{OutputRow, PurgeReport, RunRecord};
use crate::domain::scope::ScopeReport;
use crate::domain::settings::UserSettings;
use crate::domain::manifest::{CommandPreview, InstallMethod, InstallSpec, ParamValue, ToolManifest};
use crate::domain::process::{CancellationToken, InstallPlan, OutputSink, OutputStream, ProcessSpec, ProcessStatus};
use crate::domain::{AppResult, LabId, OperationId, ProcessId, ToolId};
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
    /// SHA-256 of the manifest file as loaded. Recorded in the audit log so an install can be tied to the exact definition used.
    #[serde(default)]
    pub digest: String,
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

#[async_trait]
pub trait EnvironmentDetector: Send + Sync {
    /// Component view (winget / WSL 2 / container engine) used by dependency resolution.
    async fn detect(&self) -> EnvironmentReport;
}

/// Full, honest description of the machine: facts, readiness and proposed (never performed) setup.
#[async_trait]
pub trait PlatformDetector: Send + Sync {
    async fn detect(&self) -> PlatformReport;
}

/// Non-process facts about the Windows host. A trait so detection is testable with mocks.
pub trait HostFacts: Send + Sync {
    /// (build number, display version such as "24H2", edition id)
    fn windows(&self) -> (Option<u32>, Option<String>, Option<String>);
    fn architecture(&self) -> String;
    /// Hardware virtualization enabled in firmware.
    fn firmware_virtualization(&self) -> Option<bool>;
    fn hypervisor_present(&self) -> Option<bool>;
    /// (free bytes, total bytes) of the volume containing `path`.
    fn disk(&self, path: &std::path::Path) -> Option<(u64, u64)>;
    fn pending_restart(&self) -> Option<bool>;
    /// Proxy `host:port` if configured; never credentials.
    fn proxy(&self) -> Option<String>;
    fn policy_keys(&self) -> Vec<String>;
    fn is_elevated(&self) -> Option<bool>;
    /// Value of the `DOCKER_HOST` environment variable, if set.
    fn docker_host_env(&self) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElevatedOutcome {
    pub exit_code: Option<i32>,
    /// The user declined the Windows administrator (UAC) prompt; nothing was changed.
    pub declined: bool,
}

/// Runs ONE fixed command with administrator rights after the OS shows its own consent prompt.
/// Output cannot be captured from an elevated process; only the exit code is returned.
#[async_trait]
pub trait ElevatedRunner: Send + Sync {
    async fn run(&self, program: &std::path::Path, args: &[String], timeout: Duration) -> AppResult<ElevatedOutcome>;
}

/// System changes proposed by the setup wizard. Every method requires `confirmed == true`.
#[async_trait]
pub trait SetupManager: Send + Sync {
    /// Run an automatic setup action as a tracked job.
    async fn run_action(&self, id: SetupActionId, confirmed: bool) -> AppResult<OperationId>;
    /// Schedule a Windows restart after `delay_secs`. Never happens without `confirmed`.
    async fn request_restart(&self, confirmed: bool, delay_secs: u32) -> AppResult<()>;
    async fn cancel_restart(&self) -> AppResult<()>;
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

/// Optional per-run settings: working directory and extra, fixed (non-secret) environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOptions {
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    /// Stop processes the child leaves behind when it exits (tool runs). Installers are left alone.
    pub kill_leftovers: bool,
    /// Upper bound on live processes in the child's tree (a fork-bomb guard). `None` = unlimited.
    pub max_processes: Option<u32>,
}

#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Runs to completion, streaming lines to `sink`. Kills the process tree on timeout or when
    /// `cancel` is triggered. Returns `Err` only if the process could not be started.
    async fn run(&self, spec: CommandSpec, sink: &dyn LineSink, cancel: &CancellationToken) -> AppResult<CommandResult>;

    /// Like `run`, with a working directory and extra environment. The default ignores the options;
    /// real executors override it.
    async fn run_in(&self, spec: CommandSpec, _opts: ExecOptions, sink: &dyn LineSink, cancel: &CancellationToken) -> AppResult<CommandResult> {
        self.run(spec, sink, cancel).await
    }
}

/// Finds executables using a freshly read system PATH plus explicit extra folders.
pub trait ExecutableLocator: Send + Sync {
    /// Folders passed in `extra_dirs` (install record, known folders) win over the PATH.
    fn locate(&self, file_name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf>;
    /// A Windows system program (wsl.exe, shutdown.exe, ...) from the system folder only, never from the PATH.
    /// Use this for anything that may be elevated.
    fn locate_system(&self, file_name: &str) -> Option<PathBuf>;
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
    /// Output rows after sequence number `after` (0 = from the start), with their stream, oldest first.
    fn output_since(&self, id: OperationId, after: i64, limit: usize) -> AppResult<Vec<OutputRow>>;
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

/// What a launch would do, shown before the user approves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPreview {
    pub command: CommandPreview,
    /// Present for container profiles: image, user, network, mounts, ports, limits, warnings.
    pub container: Option<RunSummary>,
    /// Who the run would talk to, and whether that is in scope. Present when the profile has network targets.
    pub scope: Option<ScopeReport>,
    pub network_activity: crate::domain::manifest::NetworkActivity,
    pub timeout_secs: u32,
    pub output_format: crate::domain::manifest::OutputFormat,
    pub expected_output: String,
    /// Where the run starts and may write result files.
    pub working_dir: String,
    /// Plain-language reminders (privileges, limits of the scope check, ...).
    pub notes: Vec<String>,
}

#[async_trait]
pub trait ToolRunner: Send + Sync {
    /// Validate parameters, approvals and scope, and describe the exact run without starting it.
    /// Touches the network only to resolve host names (and, for plain http URLs already in scope, to follow redirects).
    async fn preview(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &RunApprovals) -> AppResult<RunPreview>;
    /// Start the run as a tracked job (output, state and cancellation go through the job system).
    async fn start(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &RunApprovals) -> AppResult<OperationId>;
}

/// Durable run history (one record per launch), kept separately from raw output so output can expire sooner.
pub trait RunHistory: Send + Sync {
    fn put_run(&self, run: &RunRecord) -> AppResult<()>;
    fn get_run(&self, id: OperationId) -> AppResult<RunRecord>;
    fn list_runs(&self, limit: usize) -> AppResult<Vec<RunRecord>>;
    fn set_result_path(&self, id: OperationId, path: &str) -> AppResult<()>;
    /// Delete history older than `history_before` and stored output older than `output_before` (RFC 3339).
    fn purge(&self, history_before: &str, output_before: &str) -> AppResult<PurgeReport>;
}

/// Current user settings (lab allow-list, retention). Read on every run so changes apply immediately.
pub trait SettingsProvider: Send + Sync {
    fn get(&self) -> UserSettings;
    fn update(&self, settings: UserSettings) -> AppResult<()>;
}

/// Values for `{lab:...}` placeholders in profile defaults.
pub trait LabInfo: Send + Sync {
    fn placeholders(&self) -> HashMap<String, String>;
}

/// One plain `HEAD` request to learn where a URL redirects. Never follows the redirect itself: the caller
/// decides, hop by hop, whether the next destination is in scope.
#[async_trait]
pub trait RedirectProbe: Send + Sync {
    /// `Ok(Some(location))` for a redirect; `Ok(None)` if the response is not a redirect or cannot be read.
    async fn head(&self, url: &str) -> AppResult<Option<String>>;
}

/// Where a user-chosen save location comes from and where bytes go. The webview never supplies a path.
#[async_trait]
pub trait ResultSaver: Send + Sync {
    /// Ask the user for a file name (native dialog) and write `content`. `Ok(None)` if they cancelled.
    async fn save(&self, suggested_name: &str, content: &str) -> AppResult<Option<String>>;
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
    /// Show an existing folder in Windows Explorer. The caller passes a path it produced itself, never one from the UI.
    fn open_folder(&self, path: &std::path::Path) -> AppResult<()>;
}

// ------------------------------------------------------------------ activity

pub trait ActivityLogger: Send + Sync {
    /// Implementations must redact secrets before persisting.
    fn record(&self, event: ActivityEvent) -> AppResult<()>;
    fn recent(&self, limit: usize) -> AppResult<Vec<ActivityEvent>>;
}




// ------------------------------------------------------------------ learning

pub trait LearningCatalog: Send + Sync {
    fn list(&self) -> Vec<crate::domain::learning::Tutorial>;
    fn get(&self, id: &ToolId) -> AppResult<crate::domain::learning::Tutorial>;
}

// ------------------------------------------------------------------ AI gateway

/// Where API keys live. Implementations use the operating system's credential protection and fail closed: if it is not
/// available they return an error; they never fall back to files, the database or settings.
pub trait CredentialStore: Send + Sync {
    fn set(&self, provider: &str, secret: &crate::domain::secret::Secret) -> crate::domain::ai::AiResult<()>;
    fn get(&self, provider: &str) -> crate::domain::ai::AiResult<Option<crate::domain::secret::Secret>>;
    /// `true` if a credential existed.
    fn delete(&self, provider: &str) -> crate::domain::ai::AiResult<bool>;
    fn exists(&self, provider: &str) -> crate::domain::ai::AiResult<bool>;
    /// Plain-words name of the protection in use, shown in the UI.
    fn backend_name(&self) -> &'static str;
}

/// Receives the pieces of a response as they arrive.
pub trait StreamSink: Send + Sync {
    fn event(&self, event: &crate::domain::ai::StreamEvent);
}

/// One hosted-model provider. Adapters translate the normalized request into the provider's own API and back.
#[async_trait]
pub trait AiProvider: Send + Sync {
    fn descriptor(&self) -> crate::domain::ai::ProviderDescriptor;
    /// Provider-specific checks on a configuration (ranges the provider enforces, extra settings).
    fn validate_config(&self, config: &crate::domain::ai::ProviderConfig) -> crate::domain::ai::AiResult<()>;
    /// Provider-specific checks on a request before anything is sent (unsupported features, parameter ranges).
    fn validate_request(&self, request: &crate::domain::ai::ChatRequest) -> crate::domain::ai::AiResult<()>;
    /// The cheapest authenticated request the provider offers; succeeds only if the key is accepted.
    async fn check_auth(&self, key: &crate::domain::secret::Secret, timeout: Duration, cancel: &CancellationToken) -> crate::domain::ai::AiResult<()>;
    /// The provider's own model list. `Unsupported` if it has none.
    async fn list_models(&self, key: &crate::domain::secret::Secret, timeout: Duration, cancel: &CancellationToken) -> crate::domain::ai::AiResult<Vec<crate::domain::ai::ModelInfo>>;
    /// Send the conversation and stream the answer. Must stop promptly when `cancel` fires, never retry by itself, and never
    /// put the key anywhere but the authentication header.
    async fn chat_stream(
        &self,
        key: &crate::domain::secret::Secret,
        request: &crate::domain::ai::ChatRequest,
        config: &crate::domain::ai::ProviderConfig,
        sink: &dyn StreamSink,
        cancel: &CancellationToken,
    ) -> crate::domain::ai::AiResult<crate::domain::ai::ChatOutcome>;
}

/// Conversations, messages and non-secret provider settings.
pub trait AiStore: Send + Sync {
    fn get_config(&self, provider: &str) -> AppResult<Option<crate::domain::ai::ProviderConfig>>;
    fn put_config(&self, config: &crate::domain::ai::ProviderConfig) -> AppResult<()>;
    fn delete_config(&self, provider: &str) -> AppResult<()>;
    fn get_status(&self, provider: &str) -> AppResult<crate::domain::ai::ProviderStatus>;
    fn put_status(&self, provider: &str, status: &crate::domain::ai::ProviderStatus) -> AppResult<()>;
    /// A masked, non-secret description of the saved key (recognised prefix and length), shown in the UI.
    fn get_key_hint(&self, provider: &str) -> AppResult<Option<String>>;
    fn put_key_hint(&self, provider: &str, hint: Option<&str>) -> AppResult<()>;
    fn get_settings(&self) -> AppResult<crate::domain::ai::AiSettings>;
    fn put_settings(&self, s: &crate::domain::ai::AiSettings) -> AppResult<()>;
    fn active_provider(&self) -> AppResult<Option<String>>;
    fn set_active_provider(&self, provider: Option<&str>) -> AppResult<()>;

    fn create_conversation(&self, c: &crate::domain::ai::Conversation) -> AppResult<()>;
    fn update_conversation(&self, c: &crate::domain::ai::Conversation) -> AppResult<()>;
    fn get_conversation(&self, id: &str) -> AppResult<crate::domain::ai::Conversation>;
    fn list_conversations(&self, limit: usize) -> AppResult<Vec<crate::domain::ai::Conversation>>;
    fn delete_conversation(&self, id: &str) -> AppResult<()>;
    fn delete_all_conversations(&self) -> AppResult<usize>;

    /// Stores the message with the next sequence number of its conversation and returns it.
    fn append_message(&self, m: &crate::domain::ai::StoredMessage) -> AppResult<crate::domain::ai::StoredMessage>;
    fn update_message(&self, m: &crate::domain::ai::StoredMessage) -> AppResult<()>;
    fn list_messages(&self, conversation_id: &str) -> AppResult<Vec<crate::domain::ai::StoredMessage>>;
    fn delete_message(&self, id: &str) -> AppResult<()>;

    /// Delete conversations last touched before `cutoff` (RFC 3339). Returns how many.
    fn purge_conversations_before(&self, cutoff: &str) -> AppResult<usize>;
    /// After a restart nothing is streaming: any message still marked so was cut short.
    fn close_streaming_messages(&self) -> AppResult<usize>;

    /// The structured form of an assistant answer, kept beside its message (and deleted with it).
    fn put_turn(&self, message_id: &str, conversation_id: &str, json: &str) -> AppResult<()>;
    /// `(message id, json)` for every structured answer in a conversation.
    fn list_turns(&self, conversation_id: &str) -> AppResult<Vec<(String, String)>>;
}

/// Delivers streaming events to the user interface.
pub trait AiEventSink: Send + Sync {
    fn emit(&self, event: crate::domain::ai::AiUiEvent);
}

// ------------------------------------------------------------------ tool-aware assistant

/// Everything the assistant may know about the tools, through one narrow door. Implemented over the real registry, installer and
/// runner; tests use a fake. There is no method that installs, runs or deletes anything.
#[async_trait]
pub trait ToolFacts: Send + Sync {
    fn search(&self, query: &ToolQuery) -> Vec<CatalogEntry>;
    fn entry(&self, id: &ToolId) -> AppResult<CatalogEntry>;
    fn install_state(&self, id: &ToolId) -> Option<crate::domain::install_state::InstallState>;
    fn installed(&self, id: &ToolId) -> Option<crate::domain::jobs::InstalledTool>;
    async fn install_plan(&self, id: &ToolId) -> AppResult<InstallPlan>;
    /// Validates and describes a run. Must not start anything.
    async fn preview_run(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>) -> AppResult<RunPreview>;
    fn run_record(&self, run: OperationId) -> AppResult<crate::domain::runs::RunRecord>;
    /// Stored output lines of a finished run, at most `max_lines`.
    fn run_output(&self, run: OperationId, max_lines: usize) -> AppResult<Vec<String>>;
}

/// A public vulnerability database.
#[async_trait]
pub trait VulnSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn query_package(&self, q: &crate::domain::assistant::packages::PackageQuery, cancel: &CancellationToken) -> crate::domain::ai::AiResult<crate::domain::assistant::vuln::VulnLookup>;
    async fn get_by_id(&self, id: &str, cancel: &CancellationToken) -> crate::domain::ai::AiResult<crate::domain::assistant::vuln::VulnLookup>;
}

// ------------------------------------------------------------------ discovering packages beyond the reviewed catalog

/// Searches a public package repository. Read-only: nothing is downloaded or installed.
#[async_trait]
pub trait PackageDiscovery: Send + Sync {
    async fn search(&self, query: &str, cancel: &CancellationToken) -> AppResult<Vec<crate::domain::discover::DiscoveredPackage>>;
    async fn details(&self, id: &str, cancel: &CancellationToken) -> AppResult<crate::domain::discover::DiscoveredDetails>;
}
