//! Runs assistant actions. Every call is checked here, whoever asked: the shape of the arguments, whether the tool and profile
//! exist, and whether the *caller* may do this. A model is never allowed to do more than read the catalog; anything that touches
//! the network, local data or a run needs a person (the interface calls with `Origin::User`) and, for lookups, a standing consent.
//! No action installs software, starts a process or edits a file.

use crate::domain::assistant::actions::{Action, Risk};
use crate::domain::assistant::model::{ToolRecord, ToolStatusView};
use crate::domain::assistant::vuln::{ids_in, VulnLookup};
use crate::domain::manifest::{Category, NetworkActivity};
use crate::domain::process::CancellationToken;
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::{ToolFacts, ToolQuery, VulnSource};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_SCAN_LINES: usize = 400;
pub const MAX_SCAN_CHARS: usize = 30_000;
const SEARCH_DEFAULT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Proposed by a model. May only read the application's own catalog records.
    Model,
    /// Chosen by the person in the interface.
    User,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Grants {
    /// The person agreed that package names and versions may be sent to public vulnerability databases.
    pub external_lookup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerequisiteItem {
    pub name: String,
    pub reason: String,
    /// `None` when the computer could not be checked.
    pub satisfied: Option<bool>,
    pub requires_admin: bool,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerequisiteReport {
    pub tool_id: String,
    pub tool_name: String,
    /// True only when nothing is missing and the tool can be installed here.
    pub ready: bool,
    pub items: Vec<PrerequisiteItem>,
    pub missing: Vec<String>,
    pub dependencies: Vec<String>,
    pub administrator_rights: String,
    pub unsupported_reason: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedRun {
    pub tool_id: String,
    pub tool_name: String,
    pub profile_id: String,
    pub profile_label: String,
    pub command: String,
    pub network_activity: NetworkActivity,
    pub timeout_secs: u32,
    pub notes: Vec<String>,
    /// Always true: preparing never starts anything.
    pub requires_user_to_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanExcerpt {
    pub run_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub profile_label: String,
    pub started_at: String,
    pub lines: Vec<String>,
    pub total_lines: usize,
    pub withheld_lines: usize,
    /// Values masked inside lines that were kept (passwords, tokens, private keys, access keys…). Best effort.
    pub redacted_values: usize,
    pub truncated: bool,
    /// Vulnerability identifiers that appear in the output (upper-case). These are the only ones that can be called "scanner findings".
    pub scanner_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionOutcome {
    Catalog { records: Vec<ToolRecord> },
    ToolDetails { record: Box<ToolRecord> },
    Prerequisites { report: PrerequisiteReport },
    Installation { tool_id: String, status: ToolStatusView },
    Prepared { run: PreparedRun },
    ScanOutput { excerpt: ScanExcerpt },
    Vulnerabilities { lookup: VulnLookup },
}

pub struct ActionExecutor {
    facts: Arc<dyn ToolFacts>,
    vuln: Arc<dyn VulnSource>,
}

fn tool_id(s: &str) -> AppResult<ToolId> {
    ToolId::new(s.to_string())
}

impl ActionExecutor {
    pub fn new(facts: Arc<dyn ToolFacts>, vuln: Arc<dyn VulnSource>) -> Self {
        Self { facts, vuln }
    }

    fn record(&self, id: &ToolId) -> AppResult<ToolRecord> {
        let e = self.facts.entry(id)?;
        Ok(ToolRecord::from_manifest(&e.manifest, self.facts.install_state(id), self.facts.installed(id).as_ref()))
    }

    /// The authorization decision. Kept in one place so tests can pin it down.
    pub fn authorize(action: &Action, origin: Origin, grants: &Grants) -> AppResult<()> {
        match (action.risk(), origin) {
            (Risk::ReadOnly, _) => Ok(()),
            (_, Origin::Model) => Err(AppError::ApprovalRequired(format!("'{}' can only be started by you in the interface; the assistant can only suggest it", action.name()))),
            (Risk::ExternalLookup, Origin::User) if !grants.external_lookup => Err(AppError::ApprovalRequired("looking up vulnerabilities sends the package name and version to a public database; allow that in Settings → AI Providers first".into())),
            (_, Origin::User) => Ok(()),
        }
    }

    pub async fn execute(&self, action: &Action, origin: Origin, grants: &Grants, cancel: &CancellationToken) -> AppResult<ActionOutcome> {
        action.validate()?;
        Self::authorize(action, origin, grants)?;
        match action {
            Action::SearchCatalog { query, category, limit } => {
                let found = self.facts.search(&ToolQuery { text: Some(query.clone()), category: category.map(|c: Category| c) });
                let records = found.iter().take(usize::from(limit.unwrap_or(SEARCH_DEFAULT as u8))).map(|e| ToolRecord::from_manifest(&e.manifest, self.facts.install_state(&e.manifest.id), self.facts.installed(&e.manifest.id).as_ref())).collect();
                Ok(ActionOutcome::Catalog { records })
            }
            Action::GetToolDetails { tool_id: t } => Ok(ActionOutcome::ToolDetails { record: Box::new(self.record(&tool_id(t)?)?) }),
            Action::GetInstallationStatus { tool_id: t } => {
                let r = self.record(&tool_id(t)?)?;
                Ok(ActionOutcome::Installation { tool_id: r.id, status: r.status })
            }
            Action::CheckPrerequisites { tool_id: t } => {
                let id = tool_id(t)?;
                let r = self.record(&id)?;
                let plan = self.facts.install_plan(&id).await?;
                Ok(ActionOutcome::Prerequisites {
                    report: PrerequisiteReport {
                        tool_id: r.id,
                        tool_name: r.name,
                        ready: plan.is_ready(),
                        items: plan.prerequisites.iter().map(|p| PrerequisiteItem { name: format!("{:?}", p.spec.kind), reason: p.spec.reason.clone(), satisfied: p.satisfied, requires_admin: p.spec.requires_admin, requires_restart: p.spec.requires_restart }).collect(),
                        missing: plan.missing_prerequisites.iter().map(|m| format!("{m:?}")).collect(),
                        dependencies: plan.dependencies.iter().map(|d| d.as_str().to_string()).collect(),
                        administrator_rights: format!("{:?}", plan.elevation),
                        unsupported_reason: plan.unsupported_reason.clone(),
                        notes: plan.notes.clone(),
                    },
                })
            }
            Action::PrepareToolRun { tool_id: t, profile_id, values } => {
                let id = tool_id(t)?;
                let r = self.record(&id)?;
                let profile = r.profile(profile_id).ok_or_else(|| AppError::NotFound(format!("launch profile '{profile_id}' of {}", r.name)))?.clone();
                if !r.status.installed {
                    return Err(AppError::PrerequisiteMissing(format!("{} is not installed yet. Install it first; Vixalune only prepares runs of installed tools.", r.name)));
                }
                // The runner validates every value (types, ranges, protected folders, scope rules) and describes the command. It starts nothing.
                let preview = self.facts.preview_run(&id, profile_id, values).await?;
                Ok(ActionOutcome::Prepared {
                    run: PreparedRun {
                        tool_id: r.id,
                        tool_name: r.name,
                        profile_id: profile.id,
                        profile_label: profile.label,
                        command: preview.command.display,
                        network_activity: preview.network_activity,
                        timeout_secs: preview.timeout_secs,
                        notes: preview.notes,
                        requires_user_to_start: true,
                    },
                })
            }
            Action::ExplainScanResult { run_id } => {
                let run = OperationId(uuid::Uuid::parse_str(run_id).map_err(|_| AppError::Validation("the run id is not valid".into()))?);
                Ok(ActionOutcome::ScanOutput { excerpt: self.scan_excerpt(run)? })
            }
            Action::QuerySupportedVulnerabilitySources { ecosystem, package, version, vulnerability_id } => {
                let lookup = match (vulnerability_id, version) {
                    (Some(id), _) => self.vuln.get_by_id(id, cancel).await,
                    (None, Some(v)) => {
                        let eco = crate::domain::assistant::packages::Ecosystem::parse(ecosystem).ok_or_else(|| AppError::Unsupported("that ecosystem".into()))?;
                        self.vuln.query_package(&crate::domain::assistant::packages::PackageQuery { ecosystem: eco, name: package.clone(), version: v.clone() }, cancel).await
                    }
                    (None, None) => return Err(AppError::Validation("a version is required".into())),
                }
                .map_err(AppError::Ai)?;
                Ok(ActionOutcome::Vulnerabilities { lookup })
            }
        }
    }

    /// Stored output of a finished run, bounded, with lines that look like credentials withheld. The excerpt is *data*; what is done
    /// with it (including sending it to a model) is a separate decision by the person.
    pub fn scan_excerpt(&self, run: OperationId) -> AppResult<ScanExcerpt> {
        let rec = self.facts.run_record(run)?;
        if rec.outcome.is_none() {
            return Err(AppError::Busy("that run is still going; wait for it to finish".into()));
        }
        let all = self.facts.run_output(run, MAX_SCAN_LINES + 1)?;
        if all.is_empty() {
            return Err(AppError::NotFound("stored output for this run (it may have expired under your retention settings)".into()));
        }
        let mut truncated = all.len() > MAX_SCAN_LINES;
        let mut lines = Vec::new();
        let (mut withheld, mut chars, mut redacted) = (0usize, 0usize, 0usize);
        // Multi-line secrets (private key blocks) are masked on the whole text first, then lines are examined one by one.
        let (joined, n_block) = crate::domain::ai::redact_secrets_in_text(&all.iter().take(MAX_SCAN_LINES).cloned().collect::<Vec<_>>().join("\n"));
        redacted += n_block;
        for l in joined.split('\n') {
            let l = &l.to_string();
            if crate::domain::ai::looks_like_credential(l) {
                withheld += 1;
                lines.push("[line withheld: it looks like a credential]".to_string());
                continue;
            }
            chars += l.chars().count() + 1;
            if chars > MAX_SCAN_CHARS {
                truncated = true;
                break;
            }
            lines.push(l.clone());
        }
        let scanner_ids = ids_in(&lines.join("\n")).into_iter().collect();
        Ok(ScanExcerpt { run_id: run.to_string(), tool_id: rec.tool_id.as_str().to_string(), tool_name: rec.tool_name, profile_label: rec.profile_label, started_at: rec.started_at, total_lines: all.len(), lines, withheld_lines: withheld, redacted_values: redacted, truncated, scanner_ids })
    }
}
