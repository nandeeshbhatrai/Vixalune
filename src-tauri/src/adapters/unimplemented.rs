//! Phase-1 placeholders for ports whose behaviour lands in later phases. They fail loudly
//! with `AppError::NotImplemented` instead of pretending to work.

use crate::domain::install_state::InstallState;
use crate::domain::jobs::{InstalledTool, Job, ToolStateEntry};
use crate::domain::lab::{LabDefinition, LabState};
use crate::domain::manifest::{CommandPreview, ParamValue};
use crate::domain::process::{OutputSink, ProcessSpec, ProcessStatus};
use crate::domain::{AppError, AppResult, LabId, OperationId, ProcessId, RunId, ToolId};
use crate::ports::{InstallationManager, JobRegistry, LabManager, ProcessManager, ToolRunner};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub struct UnimplementedInstallationManager;
pub struct UnimplementedToolRunner;
pub struct UnimplementedProcessManager;
pub struct UnimplementedLabManager;
pub struct UnimplementedJobRegistry;

#[async_trait]
impl JobRegistry for UnimplementedJobRegistry {
    fn list(&self) -> AppResult<Vec<Job>> {
        Ok(Vec::new())
    }
    fn output(&self, id: ProcessId) -> AppResult<Vec<String>> {
        Err(AppError::NotFound(format!("job '{id}'")))
    }
    async fn cancel(&self, _: ProcessId) -> AppResult<()> {
        Err(AppError::NotImplemented("cancel job"))
    }
}

#[async_trait]
impl InstallationManager for UnimplementedInstallationManager {
    fn state(&self, _id: &ToolId) -> AppResult<InstallState> {
        Err(AppError::NotImplemented("installation state detection"))
    }
    fn states(&self) -> AppResult<Vec<ToolStateEntry>> {
        // Nothing is detected yet, so no state is reported (the UI shows "unknown").
        Ok(Vec::new())
    }
    fn installed(&self) -> AppResult<Vec<InstalledTool>> {
        Ok(Vec::new())
    }
    async fn install(&self, _: &ToolId, _: bool) -> AppResult<OperationId> {
        Err(AppError::NotImplemented("install"))
    }
    async fn cancel(&self, _: OperationId) -> AppResult<()> {
        Err(AppError::NotImplemented("cancel install"))
    }
    async fn retry(&self, _: &ToolId, _: bool) -> AppResult<OperationId> {
        Err(AppError::NotImplemented("retry install"))
    }
    async fn update(&self, _: &ToolId, _: bool) -> AppResult<OperationId> {
        Err(AppError::NotImplemented("update"))
    }
    async fn uninstall(&self, _: &ToolId, _: bool) -> AppResult<OperationId> {
        Err(AppError::NotImplemented("uninstall"))
    }
    async fn verify(&self, _: &ToolId) -> AppResult<InstallState> {
        Err(AppError::NotImplemented("verify"))
    }
}

#[async_trait]
impl ToolRunner for UnimplementedToolRunner {
    fn preview(&self, _: &ToolId, _: &HashMap<String, ParamValue>) -> AppResult<CommandPreview> {
        Err(AppError::NotImplemented("command preview"))
    }
    async fn start(&self, _: &ToolId, _: &HashMap<String, ParamValue>, _: Arc<dyn OutputSink>) -> AppResult<RunId> {
        Err(AppError::NotImplemented("run"))
    }
    async fn stop(&self, _: RunId) -> AppResult<()> {
        Err(AppError::NotImplemented("stop run"))
    }
}

#[async_trait]
impl ProcessManager for UnimplementedProcessManager {
    async fn spawn(&self, _: ProcessSpec, _: Arc<dyn OutputSink>) -> AppResult<ProcessId> {
        Err(AppError::NotImplemented("spawn process"))
    }
    async fn terminate(&self, _: ProcessId) -> AppResult<()> {
        Err(AppError::NotImplemented("terminate process"))
    }
    fn status(&self, _: ProcessId) -> AppResult<ProcessStatus> {
        Err(AppError::NotImplemented("process status"))
    }
}

#[async_trait]
impl LabManager for UnimplementedLabManager {
    fn list(&self) -> Vec<LabDefinition> {
        Vec::new()
    }
    fn state(&self, _: &LabId) -> AppResult<LabState> {
        Ok(LabState::Stopped)
    }
    async fn start(&self, _: &LabId) -> AppResult<()> {
        Err(AppError::NotImplemented("start lab"))
    }
    async fn stop(&self, _: &LabId) -> AppResult<()> {
        Err(AppError::NotImplemented("stop lab"))
    }
}
