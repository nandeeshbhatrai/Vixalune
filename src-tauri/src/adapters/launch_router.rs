//! Sends a launch request to the runner for the profile's runtime. Both runners share the same job store, so the
//! Running Jobs screen sees every run; cancellation is routed to whichever runner owns the job.

use crate::adapters::container_runner::ContainerToolRunner;
use crate::adapters::native_runner::NativeToolRunner;
use crate::adapters::run_support::RunSupport;
use crate::domain::container::RunApprovals;
use crate::domain::manifest::{ParamValue, RuntimeKind};
use crate::domain::runs::LaunchDefaults;
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::{RunPreview, ToolRegistry, ToolRunner};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub struct LaunchRouter {
    registry: Arc<dyn ToolRegistry>,
    pub container: Arc<ContainerToolRunner>,
    pub native: Arc<NativeToolRunner>,
    support: Arc<RunSupport>,
    /// At most this many runs at once, across all tools (resource exhaustion guard).
    max_concurrent: usize,
}

impl LaunchRouter {
    pub fn new(registry: Arc<dyn ToolRegistry>, container: Arc<ContainerToolRunner>, native: Arc<NativeToolRunner>, support: Arc<RunSupport>, max_concurrent: usize) -> Self {
        Self { registry, container, native, support, max_concurrent: max_concurrent.max(1) }
    }

    pub fn active_runs(&self) -> usize {
        self.native.active_count() + self.container.active_count()
    }

    /// Stop everything that is running and wait briefly for the processes to be cleaned up. Used when the app closes.
    pub async fn shutdown(&self, grace: std::time::Duration) -> usize {
        let asked = self.native.cancel_all() + self.container.cancel_all();
        let deadline = tokio::time::Instant::now() + grace;
        while self.active_runs() > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        asked
    }

    fn runtime(&self, id: &ToolId, profile: &str) -> AppResult<RuntimeKind> {
        let m = self.registry.get(id)?.manifest;
        m.profile(profile).map(|p| p.runtime).ok_or_else(|| AppError::NotFound(format!("launch profile '{profile}' of '{id}'")))
    }

    pub fn defaults(&self, id: &ToolId, profile: &str) -> AppResult<LaunchDefaults> {
        let m = self.registry.get(id)?.manifest;
        let p = m.profile(profile).ok_or_else(|| AppError::NotFound(format!("launch profile '{profile}' of '{id}'")))?;
        Ok(self.support.defaults(p))
    }

    /// Cancel a running job, whichever runner owns it.
    pub async fn cancel(&self, op: OperationId) -> AppResult<()> {
        match self.native.cancel(op).await {
            Err(AppError::NotFound(_)) => self.container.cancel(op).await,
            other => other,
        }
    }
}

#[async_trait]
impl ToolRunner for LaunchRouter {
    async fn preview(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &RunApprovals) -> AppResult<RunPreview> {
        match self.runtime(id, profile)? {
            RuntimeKind::Container => self.container.preview(id, profile, values, approvals).await,
            _ => self.native.preview(id, profile, values, approvals).await,
        }
    }

    async fn start(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>, approvals: &RunApprovals) -> AppResult<OperationId> {
        if self.active_runs() >= self.max_concurrent {
            return Err(AppError::Busy(format!("{} runs are already active (the limit is {}); wait for one to finish or cancel it", self.active_runs(), self.max_concurrent)));
        }
        match self.runtime(id, profile)? {
            RuntimeKind::Container => self.container.start(id, profile, values, approvals).await,
            _ => self.native.start(id, profile, values, approvals).await,
        }
    }
}
