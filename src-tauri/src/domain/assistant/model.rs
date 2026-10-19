//! What the model may say, and what the application does with it.
//!
//! The model is asked for one JSON object. That object is *input to validation*, never an instruction: every tool id, profile id,
//! evidence id and vulnerability identifier in it is checked against facts the application holds, and anything that does not
//! check out is dropped and reported. Installation status, versions, links and licences shown next to a recommendation come from
//! the application's own records, not from the model's words.

use super::actions::{Action, ProposedAction};
use super::untrusted::{looks_like_instruction, sanitize_untrusted};
use super::vuln::{scrub_unknown_ids, FindingClass, VulnLookup};
use crate::domain::install_state::InstallState;
use crate::domain::jobs::{InstalledTool, VerificationOutcome};
use crate::domain::manifest::{Category, NetworkActivity, RuntimeKind, ToolManifest};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub const MAX_MODEL_JSON_BYTES: usize = 100_000;
const MAX_RECOMMENDATIONS: usize = 4;

// ------------------------------------------------------------------ trusted tool records

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerequisiteView {
    pub name: String,
    pub reason: String,
    pub requires_admin: bool,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterView {
    pub name: String,
    pub label: String,
    pub required: bool,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub runtime: RuntimeKind,
    pub network_activity: NetworkActivity,
    pub parameters: Vec<ParameterView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStatusView {
    pub state: Option<InstallState>,
    /// Plain-language state, from the application's records.
    pub label: String,
    pub installed: bool,
    pub version: Option<String>,
    /// `Some(true)` only when the last verification check passed.
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceView {
    pub publisher: String,
    pub source_url: String,
    pub verified_on: String,
}

/// Everything the application knows about one catalog tool, in a form safe to show a model and a user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRecord {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub category: Category,
    pub tags: Vec<String>,
    pub platforms: Vec<RuntimeKind>,
    pub windows_min_build: u32,
    pub status: ToolStatusView,
    pub prerequisites: Vec<PrerequisiteView>,
    pub dependencies: Vec<String>,
    pub profiles: Vec<ProfileView>,
    pub limitations: Vec<String>,
    pub documentation: String,
    pub homepage: String,
    pub source_repository: Option<String>,
    pub license: String,
    pub provenance: ProvenanceView,
    pub can_install: bool,
}

pub fn state_label(s: Option<InstallState>) -> &'static str {
    match s {
        None => "Not checked yet",
        Some(InstallState::NotInstalled) => "Not installed",
        Some(InstallState::Queued) => "Install queued",
        Some(InstallState::Downloading) => "Downloading",
        Some(InstallState::Installing) => "Installing",
        Some(InstallState::Verifying) => "Verifying",
        Some(InstallState::Installed) => "Installed",
        Some(InstallState::UpdateAvailable) => "Installed (update available)",
        Some(InstallState::Uninstalling) => "Uninstalling",
        Some(InstallState::Cancelling) => "Cancelling",
        Some(InstallState::Cancelled) => "Not installed (install cancelled)",
        Some(InstallState::Failed) => "Install failed",
        Some(InstallState::VerificationFailed) => "Present, but verification failed",
    }
}

fn is_usable(s: Option<InstallState>) -> bool {
    matches!(s, Some(InstallState::Installed | InstallState::UpdateAvailable))
}

fn kind_summary(k: &crate::domain::manifest::ParamKind) -> String {
    use crate::domain::manifest::ParamKind as K;
    match k {
        K::Text { .. } => "text",
        K::Host => "host",
        K::Integer { .. } => "number",
        K::Boolean => "yes/no",
        K::Choice { .. } => "choice",
        K::Ip => "ip address",
        K::Url => "url",
        K::Port => "port",
        K::PortList { .. } => "port list",
        K::Path { .. } => "local path",
    }
    .to_string()
}

impl ToolRecord {
    pub fn from_manifest(m: &ToolManifest, state: Option<InstallState>, installed: Option<&InstalledTool>) -> Self {
        let profiles: Vec<ProfileView> = m
            .launch_profiles
            .iter()
            .map(|p| ProfileView {
                id: p.id.clone(),
                label: p.label.clone(),
                description: p.description.clone(),
                runtime: p.runtime,
                network_activity: p.network_activity,
                parameters: p.parameters.iter().map(|d| ParameterView { name: d.name.clone(), label: d.label.clone(), required: d.required, kind: kind_summary(&d.kind) }).collect(),
            })
            .collect();
        let mut limitations = Vec::new();
        if m.platforms == [RuntimeKind::Wsl] {
            limitations.push("Runs inside WSL 2, not as a Windows program.".to_string());
        }
        if m.platforms == [RuntimeKind::Container] {
            limitations.push("Runs in a container, so Docker must be installed and running.".to_string());
        }
        if profiles.iter().any(|p| p.network_activity == NetworkActivity::ProbesTargets) {
            limitations.push("Sends network traffic to the targets you name. Use it only on systems you own or are authorised to test.".to_string());
        }
        if profiles.iter().any(|p| p.network_activity == NetworkActivity::DownloadsData) {
            limitations.push("Downloads data (for example a vulnerability database) when it runs.".to_string());
        }
        if m.privileges.run != crate::domain::manifest::ElevationNeed::None {
            limitations.push("May need administrator rights to run.".to_string());
        }
        if m.maintenance == Some(crate::domain::manifest::MaintenanceStatus::Deprecated) {
            limitations.push("The upstream project is marked deprecated.".to_string());
        }
        if m.launch_profiles.is_empty() {
            limitations.push("Vixalune has no guided run profile for this tool yet; it can install and verify it, but not run it for you.".to_string());
        }
        let verified = installed.and_then(|i| i.last_verification.as_ref()).map(|v| v.outcome == VerificationOutcome::Passed);
        ToolRecord {
            id: m.id.as_str().to_string(),
            name: m.name.clone(),
            summary: m.summary.clone(),
            description: m.description.clone(),
            category: m.category,
            tags: m.tags.clone(),
            platforms: m.platforms.clone(),
            windows_min_build: m.windows.min_build,
            status: ToolStatusView { state, label: state_label(state).to_string(), installed: is_usable(state), version: installed.and_then(|i| i.version.clone()), verified },
            prerequisites: m
                .prerequisites
                .iter()
                .map(|p| PrerequisiteView { name: format!("{:?}", p.kind), reason: p.reason.clone(), requires_admin: p.requires_admin, requires_restart: p.requires_restart })
                .collect(),
            dependencies: m.dependencies.iter().map(|d| d.as_str().to_string()).collect(),
            profiles,
            limitations,
            documentation: m.documentation.clone(),
            homepage: m.homepage.clone(),
            source_repository: m.source_repository.clone(),
            license: m.license.clone(),
            provenance: ProvenanceView { publisher: m.provenance.publisher.clone(), source_url: m.provenance.source_url.clone(), verified_on: m.provenance.verified_on.clone() },
            can_install: !is_usable(state) && !matches!(state, Some(InstallState::Queued | InstallState::Downloading | InstallState::Installing | InstallState::Verifying | InstallState::Uninstalling | InstallState::Cancelling)),
        }
    }

    pub fn profile(&self, id: &str) -> Option<&ProfileView> {
        self.profiles.iter().find(|p| p.id == id)
    }
}

// ------------------------------------------------------------------ evidence

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    CatalogRecord,
    VulnerabilityDatabase,
    ScannerOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// `E1`, `E2`… valid only within one answer. The model cites these; it never supplies addresses.
    pub id: String,
    pub kind: EvidenceKind,
    pub title: String,
    pub url: Option<String>,
    /// When the information was retrieved or last verified.
    pub as_of: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct EvidenceSet {
    pub items: Vec<Evidence>,
}

impl EvidenceSet {
    pub fn add(&mut self, kind: EvidenceKind, title: String, url: Option<String>, as_of: Option<String>, detail: Option<String>) -> String {
        let id = format!("E{}", self.items.len() + 1);
        self.items.push(Evidence { id: id.clone(), kind, title, url, as_of, detail });
        id
    }

    pub fn ids(&self) -> BTreeSet<String> {
        self.items.iter().map(|e| e.id.clone()).collect()
    }

    pub fn urls(&self) -> BTreeSet<String> {
        self.items.iter().filter_map(|e| e.url.clone()).collect()
    }
}

// ------------------------------------------------------------------ the model's answer

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    DependencyVulnerability,
    ContainerScan,
    SourceCodeAnalysis,
    NetworkTraffic,
    WebApplication,
    Reconnaissance,
    ExplainTool,
    ExplainOutput,
    General,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelClarification {
    pub question: String,
    #[serde(default)]
    pub why: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecRole {
    Primary,
    Alternative,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRecommendation {
    pub tool_id: String,
    #[serde(default = "default_role")]
    pub role: RecRole,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
}

fn default_role() -> RecRole {
    RecRole::Alternative
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentedAlternative {
    pub name: String,
    #[serde(default)]
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelNoTool {
    pub explanation: String,
    #[serde(default)]
    pub documented_alternatives: Vec<DocumentedAlternative>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFindingNote {
    pub vulnerability_id: String,
    pub explanation: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelNextStep {
    pub label: String,
    #[serde(default)]
    pub tool_id: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAnswer {
    pub task_category: TaskCategory,
    #[serde(default)]
    pub user_intent: String,
    #[serde(default)]
    pub clarifications: Vec<ModelClarification>,
    #[serde(default)]
    pub recommended_tools: Vec<ModelRecommendation>,
    #[serde(default)]
    pub no_suitable_tool: Option<ModelNoTool>,
    #[serde(default)]
    pub findings_explained: Vec<ModelFindingNote>,
    #[serde(default)]
    pub next_steps: Vec<ModelNextStep>,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub limitations: Vec<String>,
    /// Suggested actions as raw JSON; each is parsed and checked on its own so one bad entry cannot spoil the answer.
    #[serde(default)]
    pub proposed_actions: Vec<serde_json::Value>,
}

/// Read the model's reply as the required object. Code fences and surrounding prose are tolerated; anything else is an error and
/// nothing from it is used.
pub fn parse_model_answer(raw: &str) -> Result<ModelAnswer, String> {
    if raw.len() > MAX_MODEL_JSON_BYTES {
        return Err("the answer was too large to check".into());
    }
    let t = raw.trim();
    let start = t.find('{').ok_or("the answer was not in the required format")?;
    let end = t.rfind('}').ok_or("the answer was not in the required format")?;
    if end <= start {
        return Err("the answer was not in the required format".into());
    }
    serde_json::from_str::<ModelAnswer>(&t[start..=end]).map_err(|e| format!("the answer did not match the required format ({})", e.to_string().chars().take(120).collect::<String>()))
}

// ------------------------------------------------------------------ the validated turn shown to the user

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardActionKind {
    ViewDetails,
    CheckPrerequisites,
    Install,
    PrepareRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardAction {
    pub kind: CardActionKind,
    pub enabled: bool,
    /// Why it is unavailable, when it is.
    pub reason: Option<String>,
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recommendation {
    pub tool_id: String,
    pub tool_name: String,
    pub role: RecRole,
    /// Written by the model. Shown labelled as such.
    pub reasons: Vec<String>,
    pub model_prerequisites: Vec<String>,
    pub model_limitations: Vec<String>,
    pub evidence: Vec<String>,
    // Everything below is from the application's records.
    pub summary: String,
    pub status: ToolStatusView,
    pub verified_prerequisites: Vec<PrerequisiteView>,
    pub verified_limitations: Vec<String>,
    pub profile_id: Option<String>,
    pub profile_label: Option<String>,
    pub documentation_url: String,
    pub license: String,
    pub provenance: ProvenanceView,
    pub actions: Vec<CardAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clarification {
    pub question: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoSuitableTool {
    pub explanation: String,
    /// Real-world tools that are not part of Vixalune's catalog. Not integrated; shown with that label.
    pub documented_alternatives: Vec<DocumentedAlternative>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingNote {
    pub vulnerability_id: String,
    pub explanation: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnKind {
    /// Backend-only: the package question lacked details, so no model was asked and nothing was looked up.
    NeedsInformation,
    /// Backend-only: a refusal or an unsupported request.
    Declined,
    Recommendation,
    Vulnerability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VulnerabilitySection {
    pub package: Option<super::packages::PackageQuery>,
    pub lookup: Option<VulnLookup>,
    /// What each kind of evidence means. Always shown with the findings.
    pub classes: Vec<String>,
    /// What was *not* assessed.
    pub not_assessed: Vec<String>,
    /// Identifiers that appear in scanner output the person shared. These are scanner findings, a different claim from a database match.
    #[serde(default)]
    pub scanner_ids: Vec<String>,
    #[serde(default)]
    pub scanner_note: Option<String>,
}

pub const NOT_ASSESSED: [&str; 2] = ["Whether your code actually reaches the vulnerable code (confirmed vulnerable configuration): not assessed.", "Whether the flaw can be exploited in your setup (demonstrated exploitability): not assessed."];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantTurn {
    pub version: u32,
    pub kind: TurnKind,
    pub task_category: TaskCategory,
    pub user_intent: String,
    pub clarifications: Vec<Clarification>,
    pub recommendations: Vec<Recommendation>,
    pub no_suitable_tool: Option<NoSuitableTool>,
    pub vulnerability: Option<VulnerabilitySection>,
    pub finding_notes: Vec<FindingNote>,
    /// The model's prose, after identifiers and links that cannot be traced to evidence were removed.
    pub explanation: String,
    pub limitations: Vec<String>,
    pub next_steps: Vec<String>,
    /// Suggestions that passed validation. None of them has run; each needs the person to choose it.
    pub proposed_actions: Vec<ProposedAction>,
    pub evidence: Vec<Evidence>,
    /// Things the application removed or refused, in plain language.
    pub warnings: Vec<String>,
    /// True when any text above (`explanation`, `reasons`, `next_steps`…) was written by a model.
    pub model_generated: bool,
    /// Set when the answer needs a consent the person has not given yet.
    #[serde(default)]
    pub consent_request: Option<ConsentRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRequest {
    /// `vulnerability_lookup`.
    pub kind: String,
    pub text: String,
}

impl AssistantTurn {
    pub fn empty(kind: TurnKind) -> Self {
        Self {
            version: 1,
            kind,
            task_category: TaskCategory::General,
            user_intent: String::new(),
            clarifications: vec![],
            recommendations: vec![],
            no_suitable_tool: None,
            vulnerability: None,
            finding_notes: vec![],
            explanation: String::new(),
            limitations: vec![],
            next_steps: vec![],
            proposed_actions: vec![],
            evidence: vec![],
            warnings: vec![],
            model_generated: false,
            consent_request: None,
        }
    }
}

// ------------------------------------------------------------------ validation

/// Facts the application holds for this turn.
pub struct TurnFacts<'a> {
    /// Tools retrieved for this question.
    pub records: &'a BTreeMap<String, ToolRecord>,
    /// Every tool id in the catalog (to tell "not in the catalog" from "not considered").
    pub catalog_ids: &'a BTreeSet<String>,
    pub evidence: &'a EvidenceSet,
    /// Evidence id for each retrieved tool.
    pub tool_evidence: &'a BTreeMap<String, String>,
    /// Identifiers present in database results or scanner output for this turn (upper-case).
    pub known_vuln_ids: &'a BTreeSet<String>,
    pub vulnerability: Option<VulnerabilitySection>,
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)\b(?:https?://|www\.)[^\s)>\]"']+"#).unwrap())
}

fn safety_claim_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(is|are|was|were|looks|seems|appears)( completely| totally| entirely| 100%)? (safe|secure|not vulnerable|free of (vulnerabilities|issues))\b|\bno (known )?(vulnerabilit(y|ies)|issues) (exist|exists|found|remain)\b|\bnothing to worry\b").unwrap())
}

fn md_image_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap())
}

/// Model text: bounded, stripped of images and of web addresses the application did not retrieve, and cleared of identifiers that
/// are not in the evidence. Returns the text and what was removed.
fn clean_text(text: &str, max: usize, facts: &TurnFacts<'_>, removed: &mut Vec<String>) -> String {
    let t = sanitize_untrusted(text, max);
    let t = md_image_re().replace_all(&t, "[image removed]").into_owned();
    let allowed = facts.evidence.urls();
    let t = url_re()
        .replace_all(&t, |c: &regex::Captures<'_>| {
            let u = c[0].trim_end_matches(['.', ',', ';']);
            if allowed.contains(u) {
                c[0].to_string()
            } else {
                removed.push(format!("a web address ({})", u.chars().take(60).collect::<String>()));
                "[link removed]".to_string()
            }
        })
        .into_owned();
    let (t, ids) = scrub_unknown_ids(&t, facts.known_vuln_ids);
    for id in ids {
        removed.push(format!("the identifier {id}"));
    }
    t
}

fn clean_list(items: &[String], max_items: usize, max: usize, facts: &TurnFacts<'_>, removed: &mut Vec<String>) -> Vec<String> {
    items.iter().take(max_items).map(|s| clean_text(s, max, facts, removed)).filter(|s| !s.trim().is_empty()).collect()
}

fn actions_for(r: &ToolRecord, profile: Option<&str>) -> Vec<CardAction> {
    let mut v = vec![CardAction { kind: CardActionKind::ViewDetails, enabled: true, reason: None, profile_id: None }, CardAction { kind: CardActionKind::CheckPrerequisites, enabled: true, reason: None, profile_id: None }];
    v.push(CardAction {
        kind: CardActionKind::Install,
        enabled: r.can_install,
        reason: if r.can_install { None } else if r.status.installed { Some("Already installed.".into()) } else { Some("An install or removal is already in progress.".into()) },
        profile_id: None,
    });
    let can_prepare = r.status.installed && profile.is_some();
    v.push(CardAction {
        kind: CardActionKind::PrepareRun,
        enabled: can_prepare,
        reason: if can_prepare {
            None
        } else if r.profiles.is_empty() {
            Some("Vixalune has no guided run profile for this tool.".into())
        } else {
            Some("Install the tool first.".into())
        },
        profile_id: profile.map(|p| p.to_string()),
    });
    v
}

/// Turn the model's answer into what the user sees, keeping only what checks out.
pub fn build_turn(answer: ModelAnswer, facts: &TurnFacts<'_>, kind: TurnKind) -> AssistantTurn {
    let mut removed: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut turn = AssistantTurn::empty(kind);
    turn.model_generated = true;
    turn.task_category = answer.task_category;
    turn.user_intent = clean_text(&answer.user_intent, 300, facts, &mut removed);
    turn.vulnerability = facts.vulnerability.clone();
    turn.evidence = facts.evidence.items.clone();

    turn.clarifications = answer
        .clarifications
        .iter()
        .take(3)
        .map(|c| Clarification { question: clean_text(&c.question, 300, facts, &mut removed), why: clean_text(&c.why, 300, facts, &mut removed) })
        .filter(|c| !c.question.trim().is_empty())
        .collect();

    let mut seen: BTreeSet<String> = BTreeSet::new();
    for rec in answer.recommended_tools.iter() {
        if turn.recommendations.len() >= MAX_RECOMMENDATIONS {
            warnings.push("Some extra recommendations were dropped.".into());
            break;
        }
        let id = rec.tool_id.trim().to_ascii_lowercase();
        let Some(record) = facts.records.get(&id) else {
            if facts.catalog_ids.contains(&id) {
                warnings.push(format!("A recommendation for '{id}' was removed: that tool exists but was not among the tools considered for this question."));
            } else {
                warnings.push(format!("A recommendation for '{}' was removed: there is no such tool in the catalog.", sanitize_untrusted(&rec.tool_id, 40)));
            }
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let profile = rec.profile_id.as_deref().and_then(|p| {
            if record.profile(p).is_some() {
                Some(p.to_string())
            } else {
                warnings.push(format!("The launch profile '{}' suggested for {} does not exist and was ignored.", sanitize_untrusted(p, 40), record.name));
                None
            }
        });
        let profile = profile.or_else(|| record.profiles.first().map(|p| p.id.clone()));
        let mut evidence: Vec<String> = Vec::new();
        for e in &rec.evidence {
            if facts.evidence.ids().contains(e) {
                if !evidence.contains(e) {
                    evidence.push(e.clone());
                }
            } else {
                warnings.push(format!("A citation ({}) that is not in the retrieved evidence was removed.", sanitize_untrusted(e, 12)));
            }
        }
        if let Some(own) = facts.tool_evidence.get(&id) {
            if !evidence.contains(own) {
                evidence.insert(0, own.clone());
            }
        }
        let profile_label = profile.as_deref().and_then(|p| record.profile(p)).map(|p| p.label.clone());
        turn.recommendations.push(Recommendation {
            tool_id: record.id.clone(),
            tool_name: record.name.clone(),
            role: rec.role,
            reasons: clean_list(&rec.reasons, 5, 400, facts, &mut removed),
            model_prerequisites: clean_list(&rec.prerequisites, 5, 300, facts, &mut removed),
            model_limitations: clean_list(&rec.limitations, 5, 300, facts, &mut removed),
            evidence,
            summary: record.summary.clone(),
            status: record.status.clone(),
            verified_prerequisites: record.prerequisites.clone(),
            verified_limitations: record.limitations.clone(),
            actions: actions_for(record, profile.as_deref()),
            profile_id: profile,
            profile_label,
            documentation_url: record.documentation.clone(),
            license: record.license.clone(),
            provenance: record.provenance.clone(),
        });
    }
    // Exactly one primary recommendation.
    if let Some(first_primary) = turn.recommendations.iter().position(|r| r.role == RecRole::Primary) {
        for (i, r) in turn.recommendations.iter_mut().enumerate() {
            if i != first_primary {
                r.role = RecRole::Alternative;
            }
        }
    } else if let Some(r) = turn.recommendations.first_mut() {
        r.role = RecRole::Primary;
    }

    turn.no_suitable_tool = answer.no_suitable_tool.as_ref().map(|n| NoSuitableTool {
        explanation: clean_text(&n.explanation, 600, facts, &mut removed),
        documented_alternatives: n.documented_alternatives.iter().take(4).map(|a| DocumentedAlternative { name: clean_text(&a.name, 80, facts, &mut removed), why: clean_text(&a.why, 300, facts, &mut removed) }).collect(),
    });
    if turn.recommendations.is_empty() && turn.no_suitable_tool.is_none() && turn.clarifications.is_empty() && answer.task_category != TaskCategory::ExplainTool && answer.task_category != TaskCategory::ExplainOutput && answer.task_category != TaskCategory::General {
        warnings.push("The assistant did not name a tool from the catalog for this question.".into());
    }

    for f in answer.findings_explained.iter().take(30) {
        let id = f.vulnerability_id.trim().to_ascii_uppercase();
        if !facts.known_vuln_ids.contains(&id) {
            warnings.push(format!("An explanation for '{}' was removed: that identifier is not in the vulnerability data retrieved for this question.", sanitize_untrusted(&f.vulnerability_id, 40)));
            continue;
        }
        let evidence: Vec<String> = f.evidence.iter().filter(|e| facts.evidence.ids().contains(*e)).cloned().collect();
        turn.finding_notes.push(FindingNote { vulnerability_id: id, explanation: clean_text(&f.explanation, 800, facts, &mut removed), evidence });
    }

    turn.explanation = clean_text(&answer.answer, 6000, facts, &mut removed);
    turn.limitations = clean_list(&answer.limitations, 6, 300, facts, &mut removed);
    turn.next_steps = answer
        .next_steps
        .iter()
        .take(5)
        .filter_map(|s| {
            if let Some(t) = &s.tool_id {
                if !facts.records.contains_key(&t.trim().to_ascii_lowercase()) {
                    warnings.push(format!("A next step mentioning '{}' was removed: not a tool the application knows.", sanitize_untrusted(t, 40)));
                    return None;
                }
            }
            Some(clean_text(&s.label, 200, facts, &mut removed))
        })
        .filter(|s| !s.trim().is_empty())
        .collect();

    for raw in answer.proposed_actions.iter().take(6) {
        match vet_proposal(raw, facts) {
            Ok(p) => turn.proposed_actions.push(p),
            Err(why) => warnings.push(format!("A suggested action was rejected: {why}")),
        }
    }

    if !removed.is_empty() {
        removed.sort();
        removed.dedup();
        warnings.push(format!("Removed from the model's text because it could not be traced to retrieved evidence: {}.", removed.iter().take(6).cloned().collect::<Vec<_>>().join("; ")));
    }
    if turn.kind == TurnKind::Vulnerability && safety_claim_re().is_match(&answer.answer) {
        warnings.push("The model's text says the package is safe. No lookup or scan can show that: it can only show what is publicly known or what a scanner found. Read the coverage notes.".into());
    }
    if looks_like_instruction(&answer.answer) {
        warnings.push("The model's text contained instruction-like wording. It is shown as text only; nothing in it was acted on.".into());
    }
    if turn.kind == TurnKind::Vulnerability {
        turn.limitations.push(NOT_ASSESSED[0].to_string());
        turn.limitations.push(NOT_ASSESSED[1].to_string());
    }
    turn.warnings = warnings;
    turn
}

/// Check one model-suggested action against the actions the application allows and the tools it retrieved. Pure: nothing is run
/// and the file system is not touched.
pub fn vet_proposal(raw: &serde_json::Value, facts: &TurnFacts<'_>) -> Result<ProposedAction, String> {
    let action: Action = serde_json::from_value(raw.clone()).map_err(|_| "it is not one of the actions Vixalune offers, or has arguments it does not accept".to_string())?;
    action.validate().map_err(|e| e.to_string())?;
    let tool = |id: &str| facts.records.get(id).ok_or_else(|| format!("'{}' is not a tool considered for this question", sanitize_untrusted(id, 40)));
    let summary = match &action {
        Action::SearchCatalog { query, .. } => format!("Search the catalog for \"{}\"", sanitize_untrusted(query, 80)),
        Action::GetToolDetails { tool_id } => format!("Show details of {}", tool(tool_id)?.name),
        Action::CheckPrerequisites { tool_id } => format!("Check what {} needs on this computer", tool(tool_id)?.name),
        Action::GetInstallationStatus { tool_id } => format!("Check whether {} is installed", tool(tool_id)?.name),
        Action::PrepareToolRun { tool_id, profile_id, values } => {
            let t = tool(tool_id)?;
            let p = t.profile(profile_id).ok_or_else(|| format!("{} has no launch profile '{}'", t.name, sanitize_untrusted(profile_id, 40)))?;
            if let Some(k) = values.keys().find(|k| !p.parameters.iter().any(|x| &x.name == *k)) {
                return Err(format!("'{}' is not a parameter of {} / {}", sanitize_untrusted(k, 40), t.name, p.label));
            }
            format!("Prepare \"{}\" with {} (shows exactly what would run; nothing starts until you start it)", p.label, t.name)
        }
        Action::ExplainScanResult { .. } => return Err("run ids are chosen by the person, not by the assistant".into()),
        Action::QuerySupportedVulnerabilitySources { ecosystem, package, version, vulnerability_id } => match vulnerability_id {
            Some(id) => format!("Look up {id} in the public vulnerability sources"),
            None => format!("Look up known vulnerabilities for {package} {} ({ecosystem}) in OSV", version.clone().unwrap_or_default()),
        },
    };
    Ok(ProposedAction { risk: action.risk(), action, summary })
}

/// Plain-text form of a turn, kept as the message content so later questions can refer to it.
pub fn render_text(t: &AssistantTurn) -> String {
    let mut s = String::new();
    if !t.explanation.is_empty() {
        s.push_str(&t.explanation);
        s.push_str("\n\n");
    }
    for c in &t.clarifications {
        s.push_str(&format!("Question: {}\n", c.question));
    }
    for r in &t.recommendations {
        s.push_str(&format!("- {} ({}; {}): {}\n", r.tool_name, if r.role == RecRole::Primary { "primary" } else { "alternative" }, r.status.label, r.reasons.first().cloned().unwrap_or_default()));
    }
    if let Some(v) = &t.vulnerability {
        if let Some(l) = &v.lookup {
            s.push_str(&format!("Database results from {}: {} record(s).\n", l.source, l.records.len()));
            for r in l.records.iter().take(10) {
                s.push_str(&format!("- {} {}\n", r.id, r.summary));
            }
        }
    }
    s.trim().to_string()
}

pub fn class_descriptions() -> Vec<String> {
    [FindingClass::DatabaseMatch, FindingClass::ScannerFinding].iter().map(|c| c.describe().to_string()).collect()
}
