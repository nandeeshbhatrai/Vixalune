// Tool-aware assistant. Mirrors src-tauri/src/domain/assistant/*. Status, versions, links and licences come from the
// application's own records; only fields marked "model" are written by a model.

export interface AiToolStatus {
  state: string | null;
  label: string;
  installed: boolean;
  version: string | null;
  verified: boolean | null;
}

export interface AiPrerequisite {
  name: string;
  reason: string;
  requires_admin: boolean;
  requires_restart: boolean;
}

export type CardActionKind = "view_details" | "check_prerequisites" | "install" | "prepare_run";

export interface CardAction {
  kind: CardActionKind;
  enabled: boolean;
  reason: string | null;
  profile_id: string | null;
}

export interface AssistantRecommendation {
  tool_id: string;
  tool_name: string;
  role: "primary" | "alternative";
  /** model */
  reasons: string[];
  /** model */
  model_prerequisites: string[];
  /** model */
  model_limitations: string[];
  evidence: string[];
  summary: string;
  status: AiToolStatus;
  verified_prerequisites: AiPrerequisite[];
  verified_limitations: string[];
  profile_id: string | null;
  profile_label: string | null;
  documentation_url: string;
  license: string;
  provenance: { publisher: string; source_url: string; verified_on: string };
  actions: CardAction[];
}

export interface AssistantEvidence {
  id: string;
  kind: "catalog_record" | "vulnerability_database" | "scanner_output";
  title: string;
  url: string | null;
  as_of: string | null;
  detail: string | null;
}

export interface VulnRecordView {
  id: string;
  aliases: string[];
  summary: string;
  severity: { kind: string; score: string }[];
  affected: { ecosystem: string; package: string; range_type: string; introduced: string | null; fixed: string | null; last_affected: string | null }[];
  fixed_versions: string[];
  published: string | null;
  modified: string | null;
  references: { kind: string; url: string }[];
  source: string;
  source_url: string;
  class: "database_match" | "scanner_finding" | "confirmed_vulnerable_configuration" | "demonstrated_exploitability";
}

export interface VulnLookupView {
  source: string;
  query: string;
  queried_at: string;
  records: VulnRecordView[];
  truncated: boolean;
  coverage_notes: string[];
}

export interface VulnerabilitySection {
  package: { ecosystem: string; name: string; version: string } | null;
  lookup: VulnLookupView | null;
  classes: string[];
  not_assessed: string[];
  scanner_ids: string[];
  scanner_note: string | null;
}

export type AssistantAction =
  | { action: "search_catalog"; query: string; category?: string | null; limit?: number | null }
  | { action: "get_tool_details"; tool_id: string }
  | { action: "check_prerequisites"; tool_id: string }
  | { action: "get_installation_status"; tool_id: string }
  | { action: "prepare_tool_run"; tool_id: string; profile_id: string; values?: Record<string, string | number | boolean> }
  | { action: "explain_scan_result"; run_id: string }
  | { action: "query_supported_vulnerability_sources"; ecosystem: string; package: string; version?: string | null; vulnerability_id?: string | null };

export interface ProposedAction {
  action: AssistantAction;
  risk: "read_only" | "external_lookup" | "prepares" | "reads_local_data";
  summary: string;
}

export interface AssistantTurn {
  version: number;
  kind: "needs_information" | "declined" | "recommendation" | "vulnerability";
  task_category: string;
  user_intent: string;
  clarifications: { question: string; why: string }[];
  recommendations: AssistantRecommendation[];
  no_suitable_tool: { explanation: string; documented_alternatives: { name: string; why: string }[] } | null;
  vulnerability: VulnerabilitySection | null;
  finding_notes: { vulnerability_id: string; explanation: string; evidence: string[] }[];
  /** model */
  explanation: string;
  limitations: string[];
  /** model */
  next_steps: string[];
  proposed_actions: ProposedAction[];
  evidence: AssistantEvidence[];
  warnings: string[];
  model_generated: boolean;
  consent_request: { kind: string; text: string } | null;
}

export interface AssistantTurnEntry {
  message_id: string;
  turn: AssistantTurn;
}

export interface PrerequisiteReport {
  tool_id: string;
  tool_name: string;
  ready: boolean;
  items: { name: string; reason: string; satisfied: boolean | null; requires_admin: boolean; requires_restart: boolean }[];
  missing: string[];
  dependencies: string[];
  administrator_rights: string;
  unsupported_reason: string | null;
  notes: string[];
}

export interface PreparedRun {
  tool_id: string;
  tool_name: string;
  profile_id: string;
  profile_label: string;
  command: string;
  network_activity: string;
  timeout_secs: number;
  notes: string[];
  requires_user_to_start: boolean;
}

export type AssistantActionOutcome =
  | { kind: "catalog"; records: { id: string; name: string; summary: string }[] }
  | { kind: "tool_details"; record: { id: string; name: string; summary: string } }
  | { kind: "prerequisites"; report: PrerequisiteReport }
  | { kind: "installation"; tool_id: string; status: AiToolStatus }
  | { kind: "prepared"; run: PreparedRun }
  | { kind: "scan_output"; excerpt: { lines: string[] } }
  | { kind: "vulnerabilities"; lookup: VulnLookupView };

// Discover: packages beyond the reviewed catalog.
export interface DiscoveredPackage {
  id: string;
  name: string;
  version: string;
  matched: string;
}

export interface DiscoveredDetails {
  id: string;
  name: string;
  version: string;
  publisher: string;
  publisher_url: string;
  homepage: string;
  license: string;
  license_url: string;
  description: string;
  tags: string[];
  installer_type: string;
  installer_url: string;
  installer_sha256: string;
}

export interface DiscoverResult {
  package: DiscoveredPackage;
  in_catalog: string | null;
  in_catalog_is_community: boolean;
}
