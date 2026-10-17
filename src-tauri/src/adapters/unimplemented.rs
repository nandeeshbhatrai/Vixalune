//! Placeholders for ports whose behaviour lands in later phases. They fail loudly with
//! `AppError::NotImplemented` instead of pretending to work.

use crate::domain::lab::{LabDefinition, LabState};
use crate::domain::manifest::{CommandPreview, ParamValue};
use crate::domain::process::{OutputSink, ProcessSpec, ProcessStatus};
use crate::domain::{AppError, AppResult, LabId, ProcessId, RunId, ToolId};
use crate::ports::{LabManager, ProcessManager, ToolRunner};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub struct UnimplementedToolRunner;
pub struct UnimplementedProcessManager;
pub struct UnimplementedLabManager;

#[async_trait]
impl ToolRunner for UnimplementedToolRunner {
    fn preview(&self, _: &ToolId, _: &str, _: &HashMap<String, ParamValue>) -> AppResult<CommandPreview> {
        Err(AppError::NotImplemented("command preview"))
    }
    async fn start(&self, _: &ToolId, _: &str, _: &HashMap<String, ParamValue>, _: Arc<dyn OutputSink>) -> AppResult<RunId> {
        Err(AppError::NotImplemented("launching tools"))
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
