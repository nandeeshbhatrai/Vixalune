//! Application-layer interfaces ("ports"). The UI/command layer depends only on these traits;
//! platform adapters and storage implement them. Everything is `Send + Sync` so
//! implementations can be shared as `Arc<dyn Trait>` (constructor injection).

use crate::domain::activity::ActivityEvent;
use crate::domain::environment::EnvironmentReport;
use crate::domain::install_state::InstallState;
use crate::domain::lab::{LabDefinition, LabState};
use crate::domain::manifest::{CommandPreview, InstallSpec, ParamValue, ToolManifest};
use crate::domain::process::{CancellationToken, InstallPlan, OutputSink, ProcessSpec, ProcessStatus, ProgressSink};
use crate::domain::{AppResult, LabId, OperationId, ProcessId, RunId, ToolId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Choose an install method usable in `env` and list the prerequisites that are missing.
    fn resolve(&self, manifest: &ToolManifest, env: &EnvironmentReport) -> AppResult<InstallPlan>;
}

// ------------------------------------------------------------------ installation

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub detail: String,
}

pub struct InstallContext<'a> {
    pub operation: OperationId,
    pub tool_id: &'a ToolId,
    pub progress: &'a dyn ProgressSink,
    pub cancel: &'a CancellationToken,
}

/// Executes one kind of [`InstallSpec`] using structured executables and argument arrays.
#[async_trait]
pub trait InstallerAdapter: Send + Sync {
    fn supports(&self, spec: &InstallSpec) -> bool;
    async fn install(&self, ctx: &InstallContext<'_>, spec: &InstallSpec) -> AppResult<InstallOutcome>;
    async fn uninstall(&self, ctx: &InstallContext<'_>, spec: &InstallSpec) -> AppResult<InstallOutcome>;
}

/// Owns per-tool [`InstallState`] and orchestrates adapters. Elevation must be requested
/// explicitly by the caller (`allow_elevation`) and is never inferred.
#[async_trait]
pub trait InstallationManager: Send + Sync {
    fn state(&self, id: &ToolId) -> AppResult<InstallState>;
    async fn install(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn cancel(&self, op: OperationId) -> AppResult<()>;
    async fn retry(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn update(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn uninstall(&self, id: &ToolId, allow_elevation: bool) -> AppResult<OperationId>;
    async fn verify(&self, id: &ToolId) -> AppResult<InstallState>;
}

// ------------------------------------------------------------------ running

#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, spec: ProcessSpec, output: std::sync::Arc<dyn OutputSink>) -> AppResult<ProcessId>;
    /// Terminates the process and its children.
    async fn terminate(&self, id: ProcessId) -> AppResult<()>;
    fn status(&self, id: ProcessId) -> AppResult<ProcessStatus>;
}

#[async_trait]
pub trait ToolRunner: Send + Sync {
    /// Validate parameters and build the exact command without running it.
    fn preview(&self, id: &ToolId, values: &HashMap<String, ParamValue>) -> AppResult<CommandPreview>;
    async fn start(
        &self,
        id: &ToolId,
        values: &HashMap<String, ParamValue>,
        output: std::sync::Arc<dyn OutputSink>,
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

// ------------------------------------------------------------------ activity

pub trait ActivityLogger: Send + Sync {
    /// Implementations must redact secrets before persisting.
    fn record(&self, event: ActivityEvent) -> AppResult<()>;
    fn recent(&self, limit: usize) -> AppResult<Vec<ActivityEvent>>;
}
