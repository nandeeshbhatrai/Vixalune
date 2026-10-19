//! Jobs, installation records and UI read models. Everything here describes *observed* state;
//! nothing is fabricated by the core.

use super::errors::{AppError, AppResult};
use super::install_state::InstallState;
use super::manifest::{InstallMethod, RuntimeKind};
use super::{OperationId, ToolId};
use serde::{Deserialize, Serialize};

/// Which backend features exist in this build. The UI disables and labels anything false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub install: bool,
    pub uninstall: bool,
    pub verify: bool,
    pub launch: bool,
    pub jobs: bool,
    pub labs: bool,
    /// The core can tell whether a catalog tool is installed.
    pub installed_detection: bool,
    /// Install methods that have a working adapter in this build.
    pub install_methods: Vec<InstallMethod>,
}

impl Capabilities {
    pub fn none() -> Self {
        Self {
            install: false,
            uninstall: false,
            verify: false,
            launch: false,
            jobs: false,
            labs: false,
            installed_detection: false,
            install_methods: Vec::new(),
        }
    }
}

/// `state` is `None` when the core cannot determine it (never guessed as "not installed").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStateEntry {
    pub tool_id: ToolId,
    pub state: Option<InstallState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Passed,
    Failed,
    /// Verification could not be performed. This is never reported as success.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationLevel {
    /// Only the package manager's record was checked.
    PackageRecord,
    /// The installed executable was run and its output checked (in addition to the record).
    Executable,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationResult {
    pub at: String,
    pub outcome: VerificationOutcome,
    pub level: VerificationLevel,
    pub detail: String,
}

/// Persisted knowledge about one tool's installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationRecord {
    pub tool_id: ToolId,
    pub state: InstallState,
    pub version: Option<String>,
    pub location: Option<String>,
    pub adapter: InstallMethod,
    pub manifest_version: String,
    pub installed_at: Option<String>,
    pub updated_at: String,
    pub last_verification: Option<VerificationResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledTool {
    pub tool_id: ToolId,
    pub version: Option<String>,
    pub location: Option<String>,
    pub runtime: RuntimeKind,
    pub state: InstallState,
    pub last_verification: Option<VerificationResult>,
}

// ------------------------------------------------------------------ jobs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Install,
    Update,
    Uninstall,
    Verify,
    Run,
    /// A system setup step (for example installing WSL), not a tool operation.
    Setup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    Queued,
    CheckingPrerequisites,
    AwaitingConsent,
    Installing,
    Verifying,
    Uninstalling,
    /// A tool is running (launched by the user).
    Running,
    Cancelling,
    RollingBack,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobPhase {
    pub const ACTIVE: [JobPhase; 9] = [
        Self::Queued,
        Self::CheckingPrerequisites,
        Self::AwaitingConsent,
        Self::Installing,
        Self::Verifying,
        Self::Uninstalling,
        Self::Running,
        Self::Cancelling,
        Self::RollingBack,
    ];

    pub fn is_active(self) -> bool {
        Self::ACTIVE.contains(&self)
    }

    pub fn is_terminal(self) -> bool {
        !self.is_active()
    }

    /// Stable string used in the database and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::CheckingPrerequisites => "checking_prerequisites",
            Self::AwaitingConsent => "awaiting_consent",
            Self::Installing => "installing",
            Self::Verifying => "verifying",
            Self::Uninstalling => "uninstalling",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::RollingBack => "rolling_back",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        [Self::Succeeded, Self::Failed, Self::Cancelled]
            .into_iter()
            .chain(Self::ACTIVE)
            .find(|p| p.as_str() == s)
            .ok_or_else(|| AppError::Database(format!("unknown job phase '{s}'")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobEvent {
    StartChecks,
    RequireConsent,
    BeginInstall,
    BeginUninstall,
    BeginVerify,
    BeginRun,
    RunDone,
    InstallDone,
    VerifyPassed,
    VerifyFailed,
    UninstallDone,
    BeginRollback,
    Fail,
    RequestCancel,
    CancelDone,
}

impl JobPhase {
    /// The only way a job's phase changes. Illegal transitions are errors, never ignored.
    pub fn apply(self, event: JobEvent) -> AppResult<Self> {
        use JobEvent as E;
        use JobPhase as P;
        let next = match (self, event) {
            (P::Queued, E::StartChecks) => P::CheckingPrerequisites,
            (P::CheckingPrerequisites, E::RequireConsent) => P::AwaitingConsent,
            (P::CheckingPrerequisites | P::AwaitingConsent, E::BeginInstall) => P::Installing,
            (P::CheckingPrerequisites | P::AwaitingConsent, E::BeginUninstall) => P::Uninstalling,
            (P::CheckingPrerequisites, E::BeginVerify) => P::Verifying,
            (P::CheckingPrerequisites | P::AwaitingConsent, E::BeginRun) => P::Running,
            (P::Running, E::RunDone) => P::Succeeded,
            (P::Installing, E::InstallDone) => P::Verifying,
            (P::Uninstalling, E::UninstallDone) => P::Succeeded,
            (P::Verifying, E::VerifyPassed) => P::Succeeded,
            (P::Verifying, E::VerifyFailed) => P::Failed,
            (P::Installing | P::Cancelling, E::BeginRollback) => P::RollingBack,
            (P::Queued | P::CheckingPrerequisites | P::AwaitingConsent | P::Installing | P::Verifying | P::Uninstalling | P::Running | P::RollingBack, E::Fail) => {
                P::Failed
            }
            (P::Queued | P::CheckingPrerequisites | P::AwaitingConsent | P::Installing | P::Verifying | P::Uninstalling | P::Running, E::RequestCancel) => {
                P::Cancelling
            }
            (P::Cancelling | P::RollingBack, E::CancelDone) => P::Cancelled,
            (from, event) => {
                return Err(AppError::InvalidTransition { from: from.as_str().into(), event: format!("{event:?}") });
            }
        };
        Ok(next)
    }
}

/// Actionable failure details attached to a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobError {
    pub code: String,
    pub message: String,
    pub recovery: String,
    /// Extra diagnostics: exit code, last output lines, missing prerequisites.
    pub detail: Option<String>,
}

impl JobError {
    pub fn from_app(e: &AppError, detail: Option<String>) -> Self {
        Self { code: e.code().into(), message: e.to_string(), recovery: e.recovery().into(), detail }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: OperationId,
    pub tool_id: ToolId,
    pub kind: JobKind,
    pub phase: JobPhase,
    /// Latest human-readable progress line.
    pub message: String,
    pub percent: Option<u8>,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
    pub allow_elevation: bool,
    pub error: Option<JobError>,
}

/// Which catalog link to open. The UI never supplies a URL, only this selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Homepage,
    Documentation,
    SourceRepository,
}

#[cfg(test)]
mod tests {
    use super::*;
    use JobEvent as E;
    use JobPhase as P;

    fn run(start: P, events: &[E]) -> AppResult<P> {
        events.iter().try_fold(start, |p, e| p.apply(*e))
    }

    #[test]
    fn happy_path_install() {
        let end = run(P::Queued, &[E::StartChecks, E::BeginInstall, E::InstallDone, E::VerifyPassed]);
        assert_eq!(end.unwrap(), P::Succeeded);
    }

    #[test]
    fn consent_path() {
        let end = run(P::Queued, &[E::StartChecks, E::RequireConsent, E::BeginInstall, E::InstallDone, E::VerifyPassed]);
        assert_eq!(end.unwrap(), P::Succeeded);
    }

    #[test]
    fn verification_failure_is_failed_never_succeeded() {
        let end = run(P::Queued, &[E::StartChecks, E::BeginInstall, E::InstallDone, E::VerifyFailed]).unwrap();
        assert_eq!(end, P::Failed);
        assert!(end.apply(E::VerifyPassed).is_err());
    }

    #[test]
    fn cancel_with_and_without_rollback() {
        assert_eq!(run(P::Installing, &[E::RequestCancel, E::CancelDone]).unwrap(), P::Cancelled);
        assert_eq!(run(P::Installing, &[E::RequestCancel, E::BeginRollback, E::CancelDone]).unwrap(), P::Cancelled);
        assert_eq!(run(P::Installing, &[E::BeginRollback, E::Fail]).unwrap(), P::Failed);
    }

    #[test]
    fn uninstall_path() {
        assert_eq!(run(P::Queued, &[E::StartChecks, E::BeginUninstall, E::UninstallDone]).unwrap(), P::Succeeded);
    }

    #[test]
    fn run_path_and_cancel() {
        assert_eq!(run(P::Queued, &[E::StartChecks, E::BeginRun, E::RunDone]).unwrap(), P::Succeeded);
        assert_eq!(run(P::Queued, &[E::StartChecks, E::BeginRun, E::RequestCancel, E::CancelDone]).unwrap(), P::Cancelled);
        assert_eq!(run(P::Queued, &[E::StartChecks, E::BeginRun, E::Fail]).unwrap(), P::Failed);
        assert!(P::Running.apply(E::VerifyPassed).is_err());
    }

    #[test]
    fn terminal_phases_accept_nothing() {
        for p in [P::Succeeded, P::Failed, P::Cancelled] {
            assert!(p.is_terminal());
            for e in [E::StartChecks, E::Fail, E::RequestCancel, E::VerifyPassed, E::CancelDone] {
                assert!(p.apply(e).is_err(), "{p:?} + {e:?}");
            }
        }
    }

    #[test]
    fn skipping_steps_is_illegal() {
        assert!(P::Queued.apply(E::BeginInstall).is_err());
        assert!(P::Queued.apply(E::VerifyPassed).is_err());
        assert!(P::Installing.apply(E::VerifyPassed).is_err());
        assert!(P::Cancelling.apply(E::RequestCancel).is_err());
    }

    #[test]
    fn phase_strings_round_trip() {
        for p in P::ACTIVE.into_iter().chain([P::Succeeded, P::Failed, P::Cancelled]) {
            assert_eq!(P::parse(p.as_str()).unwrap(), p);
        }
        assert!(P::parse("bogus").is_err());
    }
}

