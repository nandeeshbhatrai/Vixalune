//! The tool-aware assistant: turns a question into (1) deterministic understanding of the task, (2) retrieval of the relevant
//! catalog records and, for package questions, public vulnerability data, (3) one model call whose reply is only *input to
//! validation*, and (4) a validated, structured answer. Missing details, refusals and missing consent are answered by the
//! application itself, without asking a model, so nothing is guessed.

use super::context::{self, build_system_prompt, build_user_message, register_lookup_evidence, register_tool_evidence, UserTurn, DEFAULT_RETRIEVAL_LIMIT};
use super::executor::{ActionExecutor, ActionOutcome, Grants, Origin};
use crate::adapters::ai::service::{AiService, Finalized, Prepared, SendAck, SendInput, TurnFinalizer};
use crate::domain::ai::{AiError, AiErrorKind, AiSettings, Role};
use crate::domain::assistant::actions::Action;
use crate::domain::assistant::model::*;
use crate::domain::assistant::packages::{extract_package_intake, supported_ecosystem_names, MissingField, PackageIntake, PackageQuery};
use crate::domain::assistant::untrusted::Fence;
use crate::domain::assistant::vuln::VulnLookup;
use crate::domain::process::CancellationToken;
use crate::domain::{AppError, AppResult, OperationId};
use crate::ports::{AiEventSink, ToolFacts, VulnSource};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub struct AssistantService {
    ai: Arc<AiService>,
    facts: Arc<dyn ToolFacts>,
    pub executor: ActionExecutor,
}

/// Everything the finalizer needs to validate the model's reply, owned so it can outlive the call.
struct Finalizer {
    records: BTreeMap<String, ToolRecord>,
    catalog_ids: BTreeSet<String>,
    evidence: EvidenceSet,
    tool_evidence: BTreeMap<String, String>,
    known_vuln_ids: BTreeSet<String>,
    vulnerability: Option<VulnerabilitySection>,
    kind: TurnKind,
}

impl TurnFinalizer for Finalizer {
    fn finalize(&self, raw: &str) -> Finalized {
        let answer = match parse_model_answer(raw) {
            Ok(a) => a,
            Err(why) => {
                return Finalized { content: String::new(), turn_json: None, error: Some(AiError::new(AiErrorKind::Protocol, format!("{why}. Nothing from that reply was used. Try again, or choose another model."))) };
            }
        };
        let facts = TurnFacts { records: &self.records, catalog_ids: &self.catalog_ids, evidence: &self.evidence, tool_evidence: &self.tool_evidence, known_vuln_ids: &self.known_vuln_ids, vulnerability: self.vulnerability.clone() };
        let turn = build_turn(answer, &facts, self.kind);
        match serde_json::to_string(&turn) {
            Ok(json) => Finalized { content: render_text(&turn), turn_json: Some(json), error: None },
            Err(_) => Finalized { content: String::new(), turn_json: None, error: Some(AiError::new(AiErrorKind::Protocol, "the answer could not be stored")) },
        }
    }
}

fn section(package: Option<PackageQuery>, lookup: Option<VulnLookup>) -> VulnerabilitySection {
    VulnerabilitySection { package, lookup, classes: class_descriptions(), not_assessed: NOT_ASSESSED.iter().map(|s| s.to_string()).collect(), scanner_ids: vec![], scanner_note: None }
}

fn known_ids(lookup: &VulnLookup) -> BTreeSet<String> {
    lookup.records.iter().flat_map(|r| r.all_ids().map(|i| i.to_ascii_uppercase())).collect()
}

impl AssistantService {
    pub fn new(ai: Arc<AiService>, facts: Arc<dyn ToolFacts>, vuln: Arc<dyn VulnSource>) -> Arc<Self> {
        Arc::new(Self { executor: ActionExecutor::new(facts.clone(), vuln), ai, facts })
    }

    fn settings(&self) -> AppResult<AiSettings> {
        self.ai.settings()
    }

    fn earlier_user_texts(&self, conversation_id: &str) -> AppResult<Vec<String>> {
        let mut v: Vec<String> = self.ai.history(conversation_id)?.into_iter().filter(|m| m.role == Role::User).map(|m| m.content).collect();
        v.reverse();
        Ok(v)
    }

    fn local(&self, input: &SendInput, turn: AssistantTurn, events: &dyn AiEventSink) -> AppResult<SendAck> {
        let json = serde_json::to_string(&turn).map_err(|e| AppError::Validation(e.to_string()))?;
        self.ai.record_local_turn(&input.conversation_id, &input.text, &render_local(&turn), &json, events)
    }

    /// Ask a question. Returns once the question has been accepted; a model answer arrives later as events and is stored when it
    /// ends. Missing details, refusals and missing consent are answered at once by the application.
    pub async fn ask(self: &Arc<Self>, input: SendInput, events: Arc<dyn AiEventSink>) -> AppResult<SendAck> {
        self.ai.ensure_cloud_allowed()?;
        self.ai.check_user_text(&input.text)?;
        let (clean, redacted) = crate::domain::ai::redact_secrets_in_text(&input.text);
        let input = SendInput { text: clean, ..input };
        let mut ack = self.ask_clean(input, events).await?;
        ack.redacted_values = ack.redacted_values.max(redacted);
        Ok(ack)
    }

    async fn ask_clean(self: &Arc<Self>, input: SendInput, events: Arc<dyn AiEventSink>) -> AppResult<SendAck> {
        let earlier_owned = self.earlier_user_texts(&input.conversation_id)?;
        let earlier: Vec<&str> = earlier_owned.iter().map(|s| s.as_str()).collect();
        let settings = self.settings()?;
        let intake = extract_package_intake(&input.text, &earlier);

        let mut lookup: Option<VulnLookup> = None;
        let mut package: Option<PackageQuery> = None;
        let mut kind = TurnKind::Recommendation;
        match intake {
            PackageIntake::NotAPackageQuestion => {}
            PackageIntake::UnsupportedEcosystem { requested } => {
                let mut t = AssistantTurn::empty(TurnKind::Declined);
                t.task_category = TaskCategory::DependencyVulnerability;
                t.explanation = format!("I can't look up vulnerabilities for '{requested}' packages: Vixalune's vulnerability sources cover these ecosystems: {}. I won't guess, and a missing result there would not mean the package is safe. If you have a project folder, a scanner from the catalog may still be able to read its dependency files.", supported_ecosystem_names().join(", "));
                t.limitations.push("Unsupported ecosystem: no lookup was made.".into());
                return self.local(&input, t, events.as_ref());
            }
            PackageIntake::Incomplete { name, ecosystem, version, missing } => {
                let mut t = AssistantTurn::empty(TurnKind::NeedsInformation);
                t.task_category = TaskCategory::DependencyVulnerability;
                t.user_intent = "Find known vulnerabilities in a package".into();
                t.explanation = "I need a few details before I can look anything up. I won't guess them: the result depends on the exact package and version.".into();
                if let Some(n) = &name {
                    t.explanation.push_str(&format!(" So far I have: package '{n}'."));
                }
                if let Some(v) = &version {
                    t.explanation.push_str(&format!(" Version {v}."));
                }
                if let Some(e) = ecosystem {
                    t.explanation.push_str(&format!(" Ecosystem {}.", e.osv_name()));
                }
                for m in missing {
                    let (q, why) = match m {
                        MissingField::Name => ("Which package do you want me to check? (its exact name)".to_string(), "Different packages can have similar names.".to_string()),
                        MissingField::Ecosystem => (format!("Which ecosystem is it from? ({})", supported_ecosystem_names().join(", ")), "The same name can exist in several ecosystems and they are different packages.".to_string()),
                        MissingField::Version => ("Which version are you using?".to_string(), "Whether a vulnerability applies depends on the exact version.".to_string()),
                    };
                    t.clarifications.push(Clarification { question: q, why });
                }
                return self.local(&input, t, events.as_ref());
            }
            PackageIntake::Complete(q) => {
                kind = TurnKind::Vulnerability;
                if !settings.allow_vulnerability_lookup {
                    let mut t = AssistantTurn::empty(TurnKind::NeedsInformation);
                    t.task_category = TaskCategory::DependencyVulnerability;
                    t.user_intent = format!("Find known vulnerabilities in {} {} ({})", q.name, q.version, q.ecosystem.osv_name());
                    t.explanation = format!("I have what I need ({} {} from {}), but looking it up means sending the ecosystem, package name and version to the public OSV database (api.osv.dev). Nothing else is sent. Nothing was sent yet.", q.name, q.version, q.ecosystem.osv_name());
                    t.consent_request = Some(ConsentRequest { kind: "vulnerability_lookup".into(), text: format!("Allow Vixalune to send the package name and version (for example “{} {}”) to OSV and NVD to look up known vulnerabilities.", q.name, q.version) });
                    t.vulnerability = Some(section(Some(q), None));
                    return self.local(&input, t, events.as_ref());
                }
                let action = Action::QuerySupportedVulnerabilitySources { ecosystem: q.ecosystem.osv_name().into(), package: q.name.clone(), version: Some(q.version.clone()), vulnerability_id: None };
                match self.executor.execute(&action, Origin::User, &Grants { external_lookup: true }, &CancellationToken::new()).await {
                    Ok(ActionOutcome::Vulnerabilities { lookup: l }) => lookup = Some(l),
                    Ok(_) => return Err(AppError::Validation("unexpected result".into())),
                    Err(e) => {
                        // No model is asked to speculate about a database it could not read.
                        let mut t = AssistantTurn::empty(TurnKind::Declined);
                        t.task_category = TaskCategory::DependencyVulnerability;
                        t.explanation = format!("I could not read the vulnerability database ({e}). Nothing was verified, so I won't say anything about {} {}. Try again in a moment.", q.name, q.version);
                        t.limitations.push("The lookup failed: this is not a statement that the package is safe.".into());
                        t.vulnerability = Some(section(Some(q), None));
                        return self.local(&input, t, events.as_ref());
                    }
                }
                package = Some(q);
            }
        }
        self.ai.ready_for(&input.provider, &input.model)?;

        // Retrieval: the question, plus scanner wording for package questions so a compatible scanner can be offered.
        let query = if package.is_some() { format!("{} scan dependencies vulnerability", input.text) } else { input.text.clone() };
        let r = context::retrieve(self.facts.as_ref(), &query, &earlier, DEFAULT_RETRIEVAL_LIMIT);
        let mut evidence = EvidenceSet::default();
        let tool_evidence = register_tool_evidence(&mut evidence, &r.records);
        let mut known: BTreeSet<String> = BTreeSet::new();
        let mut external = Vec::new();
        if let Some(l) = &lookup {
            let ids = register_lookup_evidence(&mut evidence, l);
            known = known_ids(l);
            // The model needs identifiers, summaries, ranges, fixed versions and dates. Reference lists are shown to the person from the
            // stored records and are not sent, which keeps the request small.
            let mut slim = l.clone();
            for r in &mut slim.records {
                r.references.clear();
            }
            let payload = serde_json::json!({"evidence_ids": ids, "lookup": slim});
            external.push((if l.source == "NVD" { "NVD" } else { "OSV" }, serde_json::to_string(&payload).unwrap_or_default()));
        }
        let fence = Fence::new();
        let system = build_system_prompt(&r.records, &tool_evidence);
        let user = build_user_message(&UserTurn { question: &input.text, evidence: &evidence, external, scanner_output: None, fence: &fence });
        let fin = Finalizer {
            records: r.records.iter().map(|x| (x.id.clone(), x.clone())).collect(),
            catalog_ids: r.catalog_ids.iter().cloned().collect(),
            evidence,
            tool_evidence,
            known_vuln_ids: known,
            vulnerability: if kind == TurnKind::Vulnerability { Some(section(package, lookup)) } else { None },
            kind,
        };
        self.ai.send_prepared(input, Prepared { system: Some(system), model_user_text: Some(user), hide_deltas: true, finalizer: Some(Arc::new(fin)), title: None }, events)
    }

    /// Explain the stored output of one finished run. The person must have agreed, in the interface, to send that output to the
    /// provider; the lines sent are bounded and credential-looking lines are withheld.
    pub async fn explain_run(self: &Arc<Self>, conversation_id: &str, run_id: &str, consent: bool, provider: String, model: String, events: Arc<dyn AiEventSink>) -> AppResult<SendAck> {
        if !consent {
            return Err(AppError::ApprovalRequired("explaining a run sends part of its output to your AI provider; confirm that in the dialog first".into()));
        }
        let run = OperationId(uuid::Uuid::parse_str(run_id).map_err(|_| AppError::Validation("the run id is not valid".into()))?);
        self.ai.ensure_cloud_allowed()?;
        self.ai.ready_for(&provider, &model)?;
        let excerpt = self.executor.scan_excerpt(run)?;
        let entry = self.facts.entry(&crate::domain::ToolId::new(excerpt.tool_id.clone())?)?;
        let record = ToolRecord::from_manifest(&entry.manifest, self.facts.install_state(&entry.manifest.id), self.facts.installed(&entry.manifest.id).as_ref());
        let mut evidence = EvidenceSet::default();
        let tool_evidence = register_tool_evidence(&mut evidence, std::slice::from_ref(&record));
        evidence.add(
            EvidenceKind::ScannerOutput,
            format!("Output of the {} run “{}” ({})", excerpt.tool_name, excerpt.profile_label, excerpt.started_at),
            None,
            Some(excerpt.started_at.clone()),
            Some(format!("{} line(s) shared; {} value(s) masked and {} line(s) withheld as credential-like", excerpt.lines.len(), excerpt.redacted_values, excerpt.withheld_lines)),
        );
        let ids: BTreeSet<String> = excerpt.scanner_ids.iter().cloned().collect();
        let mut sec = section(None, None);
        sec.scanner_ids = excerpt.scanner_ids.clone();
        sec.scanner_note = Some(format!("Identifiers below are only those that appear in the output of this {} run. They are scanner findings, not independently confirmed.", excerpt.tool_name));
        let shown = format!("Explain the output of {} — {} (run started {})", excerpt.tool_name, excerpt.profile_label, excerpt.started_at);
        let fence = Fence::new();
        let system = build_system_prompt(std::slice::from_ref(&record), &tool_evidence);
        let label = format!("{} line(s){}", excerpt.lines.len(), if excerpt.truncated { ", truncated" } else { "" });
        let user = build_user_message(&UserTurn { question: &format!("{shown}. Explain what it shows and what the person could do next. Do not run anything."), evidence: &evidence, external: vec![], scanner_output: Some((label, excerpt.lines.join("\n"))), fence: &fence });
        let fin = Finalizer {
            records: BTreeMap::from([(record.id.clone(), record.clone())]),
            catalog_ids: self.facts.search(&Default::default()).iter().map(|e| e.manifest.id.as_str().to_string()).collect(),
            evidence,
            tool_evidence,
            known_vuln_ids: ids,
            vulnerability: Some(sec),
            kind: TurnKind::Vulnerability,
        };
        let input = SendInput { conversation_id: conversation_id.to_string(), text: shown, provider, model, confirm_large: false };
        self.ai.send_prepared(input, Prepared { system: Some(system), model_user_text: Some(user), hide_deltas: true, finalizer: Some(Arc::new(fin)), title: None }, events)
    }

    /// Ask the last question again after a failed or stopped answer (an explicit user action).
    pub async fn retry(self: &Arc<Self>, conversation_id: &str, provider: String, model: String, events: Arc<dyn AiEventSink>) -> AppResult<SendAck> {
        let text = self.ai.take_last_failed_question(conversation_id)?;
        self.ask(SendInput { conversation_id: conversation_id.to_string(), text, provider, model, confirm_large: false }, events).await
    }

    /// Structured answers of a conversation, keyed by the message they belong to.
    pub fn turns(&self, conversation_id: &str) -> AppResult<Vec<(String, AssistantTurn)>> {
        Ok(self
            .ai
            .turns(conversation_id)?
            .into_iter()
            .filter_map(|(id, json)| match serde_json::from_str::<AssistantTurn>(&json) {
                Ok(t) => Some((id, t)),
                Err(e) => {
                    tracing::warn!("a stored assistant answer could not be read: {e}");
                    None
                }
            })
            .collect())
    }

    /// An action chosen by the person in the interface.
    pub async fn run_action(&self, action: Action) -> AppResult<ActionOutcome> {
        let grants = Grants { external_lookup: self.settings()?.allow_vulnerability_lookup };
        self.executor.execute(&action, Origin::User, &grants, &CancellationToken::new()).await
    }
}

/// Plain text kept as the message for turns the application wrote itself.
fn render_local(t: &AssistantTurn) -> String {
    let mut s = t.explanation.clone();
    for c in &t.clarifications {
        s.push_str(&format!("\nQuestion: {}", c.question));
    }
    s
}
