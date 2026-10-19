//! `ToolFacts` over the application's real registry, installer, runner and history.

use crate::domain::container::RunApprovals;
use crate::domain::install_state::InstallState;
use crate::domain::jobs::InstalledTool;
use crate::domain::manifest::ParamValue;
use crate::domain::process::InstallPlan;
use crate::domain::runs::RunRecord;
use crate::domain::{AppResult, OperationId, ToolId};
use crate::ports::{CatalogEntry, InstallationManager, JobStore, RunHistory, RunPreview, ToolFacts, ToolQuery, ToolRegistry, ToolRunner};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub struct AppToolFacts {
    pub registry: Arc<dyn ToolRegistry>,
    pub installer: Arc<dyn InstallationManager>,
    pub runner: Arc<dyn ToolRunner>,
    pub history: Arc<dyn RunHistory>,
    pub store: Arc<dyn JobStore>,
}

#[async_trait]
impl ToolFacts for AppToolFacts {
    fn search(&self, query: &ToolQuery) -> Vec<CatalogEntry> {
        self.registry.list(query)
    }

    fn entry(&self, id: &ToolId) -> AppResult<CatalogEntry> {
        self.registry.get(id)
    }

    fn install_state(&self, id: &ToolId) -> Option<InstallState> {
        self.installer.state(id).ok()
    }

    fn installed(&self, id: &ToolId) -> Option<InstalledTool> {
        self.installer.installed().ok()?.into_iter().find(|t| &t.tool_id == id)
    }

    async fn install_plan(&self, id: &ToolId) -> AppResult<InstallPlan> {
        self.installer.plan(id).await
    }

    /// Previewing validates the parameters and the scope rules and describes the command. It starts nothing and uses default
    /// approvals, which grant no network, mount or port permission.
    async fn preview_run(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>) -> AppResult<RunPreview> {
        self.runner.preview(id, profile, values, &RunApprovals::default()).await
    }

    fn run_record(&self, run: OperationId) -> AppResult<RunRecord> {
        self.history.get_run(run)
    }

    fn run_output(&self, run: OperationId, max_lines: usize) -> AppResult<Vec<String>> {
        self.store.job_output(run, max_lines)
    }
}
