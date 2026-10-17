//! Process, progress and install-plan models shared by the ports.

use super::ids::{OperationId, ProcessId, ToolId};
use super::manifest::{ElevationNeed, InstallSpec, IntegrityLevel, Prerequisite, PrerequisiteSpec};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A process to spawn. `program` + `args` are passed straight to the OS (no shell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    /// Environment variables to *add*. The parent environment is not forwarded wholesale.
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Running,
    Exited { code: i32 },
    Terminated,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputLine {
    pub process: ProcessId,
    pub stream: OutputStream,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub operation: OperationId,
    pub tool_id: ToolId,
    pub message: String,
    /// 0..=100 when known.
    pub percent: Option<u8>,
}

pub trait ProgressSink: Send + Sync {
    fn report(&self, event: ProgressEvent);
}

pub trait OutputSink: Send + Sync {
    fn line(&self, line: OutputLine);
}

/// Cooperative cancellation flag shared between a manager and an adapter.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One prerequisite as it applies to this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerequisiteStatus {
    pub spec: PrerequisiteSpec,
    /// `None` when the environment could not be checked.
    pub satisfied: Option<bool>,
}

/// The result of dependency resolution: what would be done, what is missing first, and what the
/// user should know before consenting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub tool_id: ToolId,
    pub method: InstallSpec,
    pub prerequisites: Vec<PrerequisiteStatus>,
    pub missing_prerequisites: Vec<Prerequisite>,
    pub dependencies: Vec<ToolId>,
    pub elevation: ElevationNeed,
    pub may_require_restart: bool,
    pub network_required: bool,
    pub approx_download_mb: Option<u32>,
    pub approx_disk_mb: Option<u32>,
    pub license: String,
    pub integrity: IntegrityLevel,
    /// Set when this tool cannot be installed here at all (Windows too old, wrong architecture,
    /// no adapter). Distinct from a missing prerequisite, which the user can fix.
    pub unsupported_reason: Option<String>,
}

impl InstallPlan {
    pub fn is_ready(&self) -> bool {
        self.missing_prerequisites.is_empty() && self.unsupported_reason.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared_between_clones() {
        let a = CancellationToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }
}
