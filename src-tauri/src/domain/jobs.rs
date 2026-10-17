//! Read models for the UI: capabilities, installed tools, tool states and jobs.
//! These describe *observed* state only; nothing here is fabricated by the core.

use super::install_state::InstallState;
use super::manifest::RuntimeKind;
use super::{ProcessId, ToolId};
use serde::{Deserialize, Serialize};

/// Which backend features exist in this build. The UI disables and labels anything false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub install: bool,
    pub uninstall: bool,
    pub verify: bool,
    pub launch: bool,
    pub jobs: bool,
    pub labs: bool,
    /// The core can tell whether a catalog tool is installed.
    pub installed_detection: bool,
}

impl Capabilities {
    /// Phase 2: only read-only features exist.
    pub const fn none() -> Self {
        Self { install: false, uninstall: false, verify: false, launch: false, jobs: false, labs: false, installed_detection: false }
    }
}

/// `state` is `None` when the core cannot determine it (never guessed as "not installed").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStateEntry {
    pub tool_id: ToolId,
    pub state: Option<InstallState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationResult {
    pub at: String,
    pub passed: bool,
    pub detail: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Install,
    Uninstall,
    Verify,
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: ProcessId,
    pub tool_id: ToolId,
    pub kind: JobKind,
    pub state: JobState,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
}

/// Which catalog link to open. The UI never supplies a URL, only this selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Homepage,
    Documentation,
    SourceRepository,
}
