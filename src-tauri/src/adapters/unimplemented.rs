//! Placeholders for ports whose behaviour lands in later phases. They fail loudly with
//! `AppError::NotImplemented` instead of pretending to work.

use crate::domain::lab::{LabDefinition, LabState};
use crate::domain::process::{OutputSink, ProcessSpec, ProcessStatus};
use crate::domain::{AppError, AppResult, LabId, ProcessId};
use crate::ports::{LabManager, ProcessManager};
use async_trait::async_trait;
use std::sync::Arc;

pub struct UnimplementedProcessManager;
pub struct UnimplementedLabManager;

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

