//! Deterministic fixtures for the assistant tests: the real catalog from `manifests/`, scripted installation state, previews that
//! never start anything, and recorded runs.

use crate::adapters::registry::ManifestRegistry;
use crate::domain::ai::{AiError, AiErrorKind, AiResult};
use crate::domain::assistant::packages::PackageQuery;
use crate::domain::assistant::vuln::{coverage_notes_for, parse_osv_vulnerability, VulnLookup};
use crate::domain::install_state::InstallState;
use crate::domain::jobs::InstalledTool;
use crate::domain::manifest::{NetworkActivity, ParamValue, RuntimeKind};
use crate::domain::process::{CancellationToken, InstallPlan, PrerequisiteStatus};
use crate::domain::runs::{RunOutcome, RunRecord};
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::{CatalogEntry, RunPreview, ToolFacts, ToolQuery, ToolRegistry, TrustLevel, VulnSource};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct FakeFacts {
    registry: ManifestRegistry,
    states: Mutex<HashMap<String, InstallState>>,
    versions: Mutex<HashMap<String, String>>,
    pub missing_prereqs: Vec<crate::domain::manifest::Prerequisite>,
    runs: Mutex<HashMap<OperationId, (RunRecord, Vec<String>)>>,
    pub previews: AtomicUsize,
    pub plans: AtomicUsize,
}

impl FakeFacts {
    pub fn from_repo_manifests() -> Self {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests");
        let registry = ManifestRegistry::load_dir(&dir, TrustLevel::Bundled).expect("the repository manifests load");
        assert!(registry.rejected.is_empty(), "{:?}", registry.rejected);
        Self { registry, states: Default::default(), versions: Default::default(), missing_prereqs: vec![], runs: Default::default(), previews: AtomicUsize::new(0), plans: AtomicUsize::new(0) }
    }

    pub fn set_installed(&self, id: &str, version: &str) {
        self.states.lock().unwrap().insert(id.into(), InstallState::Installed);
        self.versions.lock().unwrap().insert(id.into(), version.into());
    }

    pub fn set_state(&self, id: &str, s: InstallState) {
        self.states.lock().unwrap().insert(id.into(), s);
    }

    pub fn add_run(&self, tool: &str, profile: &str, output: &[&str]) -> OperationId {
        let e = self.registry.get(&ToolId::new(tool).unwrap()).unwrap();
        let id = OperationId::new();
        let label = e.manifest.launch_profiles.iter().find(|p| p.id == profile).map(|p| p.label.clone()).unwrap_or_default();
        let rec = RunRecord {
            id,
            tool_id: e.manifest.id.clone(),
            tool_name: e.manifest.name.clone(),
            profile_id: profile.into(),
            profile_label: label,
            tool_version: self.versions.lock().unwrap().get(tool).cloned(),
            started_at: "2026-10-20T10:00:00Z".into(),
            finished_at: Some("2026-10-20T10:00:09Z".into()),
            duration_ms: Some(9000),
            exit_code: Some(1),
            outcome: Some(RunOutcome::Findings),
            command: "trivy.exe fs --no-progress".into(),
            targets: vec![],
            scope: "the sample project".into(),
            network_activity: NetworkActivity::DownloadsData,
            output_format: crate::domain::manifest::OutputFormat::Text,
            run_dir: None,
            result_path: None,
        };
        self.runs.lock().unwrap().insert(id, (rec, output.iter().map(|s| s.to_string()).collect()));
        id
    }
}

#[async_trait]
impl ToolFacts for FakeFacts {
    fn search(&self, q: &ToolQuery) -> Vec<CatalogEntry> {
        self.registry.list(q)
    }
    fn entry(&self, id: &ToolId) -> AppResult<CatalogEntry> {
        self.registry.get(id)
    }
    fn install_state(&self, id: &ToolId) -> Option<InstallState> {
        Some(self.states.lock().unwrap().get(id.as_str()).copied().unwrap_or(InstallState::NotInstalled))
    }
    fn installed(&self, id: &ToolId) -> Option<InstalledTool> {
        let s = *self.states.lock().unwrap().get(id.as_str())?;
        Some(InstalledTool { tool_id: id.clone(), version: self.versions.lock().unwrap().get(id.as_str()).cloned(), location: None, runtime: RuntimeKind::WindowsNative, state: s, last_verification: None })
    }
    async fn install_plan(&self, id: &ToolId) -> AppResult<InstallPlan> {
        self.plans.fetch_add(1, Ordering::SeqCst);
        let m = self.registry.get(id)?.manifest;
        Ok(InstallPlan {
            tool_id: id.clone(),
            method: m.install[0].clone(),
            prerequisites: m.prerequisites.iter().map(|p| PrerequisiteStatus { spec: p.clone(), satisfied: Some(!self.missing_prereqs.contains(&p.kind)) }).collect(),
            missing_prerequisites: self.missing_prereqs.clone(),
            dependencies: m.dependencies.clone(),
            elevation: m.privileges.install,
            may_require_restart: false,
            network_required: m.network.install,
            approx_download_mb: None,
            approx_disk_mb: None,
            license: m.license.clone(),
            integrity: m.integrity.level,
            notes: vec![],
            unsupported_reason: None,
        })
    }
    async fn preview_run(&self, id: &ToolId, profile: &str, values: &HashMap<String, ParamValue>) -> AppResult<RunPreview> {
        self.previews.fetch_add(1, Ordering::SeqCst);
        let m = self.registry.get(id)?.manifest;
        let p = m.launch_profiles.iter().find(|p| p.id == profile).ok_or_else(|| AppError::NotFound(format!("launch profile '{profile}'")))?;
        for d in &p.parameters {
            if d.required && d.default.is_none() && !values.contains_key(&d.name) {
                return Err(AppError::Validation(format!("'{}' is required", d.name)));
            }
        }
        for k in values.keys() {
            if !p.parameters.iter().any(|d| &d.name == k) {
                return Err(AppError::Validation(format!("'{k}' is not a parameter of this profile")));
            }
        }
        Ok(RunPreview {
            command: crate::domain::manifest::CommandPreview { runtime: p.runtime, program: p.executable.clone(), args: vec![], display: format!("{} (preview)", p.executable) },
            container: None,
            scope: None,
            network_activity: p.network_activity,
            timeout_secs: p.timeout_secs.unwrap_or(600),
            output_format: p.output_format,
            expected_output: String::new(),
            working_dir: "run folder".into(),
            notes: vec![],
        })
    }
    fn run_record(&self, run: OperationId) -> AppResult<RunRecord> {
        self.runs.lock().unwrap().get(&run).map(|r| r.0.clone()).ok_or_else(|| AppError::NotFound("that run".into()))
    }
    fn run_output(&self, run: OperationId, max: usize) -> AppResult<Vec<String>> {
        self.runs.lock().unwrap().get(&run).map(|r| r.1.iter().take(max).cloned().collect()).ok_or_else(|| AppError::NotFound("that run".into()))
    }
}

/// A vulnerability source that returns what it is given and counts its calls.
pub struct FakeVulns {
    pub result: Mutex<AiResult<VulnLookup>>,
    pub calls: AtomicUsize,
    pub asked: Mutex<Vec<String>>,
}

impl FakeVulns {
    pub fn lodash() -> Self {
        let v: serde_json::Value = serde_json::from_str(include_str!("fixtures/osv_lodash_4_17_15.json")).unwrap();
        let q = PackageQuery { ecosystem: crate::domain::assistant::packages::Ecosystem::Npm, name: "lodash".into(), version: "4.17.15".into() };
        let records: Vec<_> = v["vulns"].as_array().unwrap().iter().filter_map(|x| parse_osv_vulnerability(x, Some(&q))).collect();
        Self::with(VulnLookup { source: "OSV".into(), query: "npm lodash 4.17.15".into(), queried_at: "2026-10-20T10:00:00Z".into(), coverage_notes: coverage_notes_for("OSV", records.len()), records, truncated: false })
    }
    pub fn empty() -> Self {
        Self::with(VulnLookup { source: "OSV".into(), query: "q".into(), queried_at: "2026-10-20T10:00:00Z".into(), records: vec![], truncated: false, coverage_notes: coverage_notes_for("OSV", 0) })
    }
    pub fn with(l: VulnLookup) -> Self {
        Self { result: Mutex::new(Ok(l)), calls: AtomicUsize::new(0), asked: Mutex::new(vec![]) }
    }
    pub fn failing(kind: AiErrorKind) -> Self {
        Self { result: Mutex::new(Err(AiError::new(kind, "the database could not be reached"))), calls: AtomicUsize::new(0), asked: Mutex::new(vec![]) }
    }
}

#[async_trait]
impl VulnSource for FakeVulns {
    fn name(&self) -> &'static str {
        "OSV"
    }
    async fn query_package(&self, q: &PackageQuery, _: &CancellationToken) -> AiResult<VulnLookup> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.asked.lock().unwrap().push(format!("{} {} {}", q.ecosystem.osv_name(), q.name, q.version));
        self.result.lock().unwrap().clone()
    }
    async fn get_by_id(&self, id: &str, _: &CancellationToken) -> AiResult<VulnLookup> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.asked.lock().unwrap().push(id.to_string());
        self.result.lock().unwrap().clone()
    }
}
