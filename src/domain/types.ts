// TypeScript mirror of the Rust domain wire format (snake_case, as serialized by serde).
// Keep in sync with src-tauri/src/domain and docs/TOOL_MANIFEST_SCHEMA.md.

export type RuntimeKind = "windows_native" | "wsl" | "container";

export type Category =
  | "reconnaissance"
  | "web_application"
  | "network"
  | "passwords"
  | "vulnerability"
  | "forensics"
  | "utilities";

export type Prerequisite = "winget" | "wsl2" | "docker";
export type MaintenanceStatus = "active" | "stable" | "deprecated";

export type InstallSpec =
  | { method: "winget"; package_id: string }
  | { method: "wsl_apt"; packages: string[] }
  | { method: "container_image"; image: string; digest: string }
  | { method: "download_archive"; url: string; sha256: string; executable: string };

export type InstallMethod = InstallSpec["method"];
export type ElevationNeed = "none" | "possible" | "required";
export type IntegrityLevel = "package_manager" | "sha256_pinned" | "image_digest" | "unavailable";
export type Architecture = "x64" | "arm64";

export type ParamValue = boolean | number | string;

export type PathKind = "file" | "directory";

export type ParamKind =
  | { type: "text"; max_len?: number | null }
  | { type: "host" }
  | { type: "ip" }
  | { type: "url" }
  | { type: "port" }
  | { type: "port_list"; max_items?: number | null }
  | { type: "path"; path_kind: PathKind; must_exist?: boolean; extensions?: string[] }
  | { type: "integer"; min: number; max: number }
  | { type: "boolean" }
  | { type: "choice"; options: string[] };

export interface ParameterDef {
  name: string;
  label: string;
  description: string;
  kind: ParamKind;
  required: boolean;
  default?: ParamValue | null;
}

export interface PrerequisiteSpec {
  kind: Prerequisite;
  reason: string;
  approx_size_mb?: number | null;
  requires_admin: boolean;
  requires_restart: boolean;
}

export type VerifyCheck =
  | { kind: "package_installed" }
  | {
      kind: "executable_output";
      runtime: RuntimeKind;
      executable: string;
      args: string[];
      locations: string[];
      expect_output_contains?: string | null;
    };

export type OutputFormat = "text" | "json" | "xml" | "html" | "sarif";
export type NetworkActivity = "none" | "probes_targets" | "downloads_data";

export interface RunSpec {
  id: string;
  label: string;
  description: string;
  runtime: RuntimeKind;
  executable: string;
  parameters: ParameterDef[];
  /** Container profiles: names from `container.mounts` the user must supply a folder for. */
  mounts: string[];
  needs_network: boolean;
  ports: number[];
  working_dir?: "run_dir" | "install_dir";
  timeout_secs?: number | null;
  output_format?: OutputFormat;
  network_activity?: NetworkActivity;
  exit_codes?: { ok: number[]; findings: number[] };
  expected_output?: string;
  suggested_mounts?: { name: string; value: string }[];
}

export interface ContainerPolicy {
  user: string;
  read_only_rootfs: boolean;
  memory_mb: number;
  cpus_milli: number;
  pids_limit: number;
  tmpfs: string[];
  workdir?: string | null;
  env: { name: string; value: string }[];
  mounts: { name: string; container_path: string; read_only: boolean }[];
  ports: number[];
}

export interface ToolManifest {
  schema_version: number;
  manifest_version: string;
  maintainer: string;
  id: string;
  name: string;
  summary: string;
  description: string;
  category: Category;
  tags: string[];
  license: string;
  homepage: string;
  documentation: string;
  source_repository?: string | null;
  provenance: {
    publisher: string;
    basis: "official_upstream" | "package_repository" | "community_maintained";
    source_url: string;
    verified_on: string;
  };
  maintenance?: MaintenanceStatus | null;
  recommended_for_beginners: boolean;
  windows: { min_build: number; architectures: Architecture[] };
  platforms: RuntimeKind[];
  prerequisites: PrerequisiteSpec[];
  dependencies: string[];
  install: InstallSpec[];
  version: { pinned?: string | null; minimum?: string | null };
  storage: { install_subdir?: string | null; approx_download_mb?: number | null; approx_disk_mb?: number | null };
  integrity: { level: IntegrityLevel; note?: string | null };
  network: { install: boolean; run: boolean };
  privileges: { install: ElevationNeed; run: ElevationNeed; may_require_restart: boolean };
  lifecycle: { can_update: boolean; can_uninstall: boolean };
  verify: { timeout_secs?: number | null; checks: VerifyCheck[] };
  container?: ContainerPolicy | null;
  launch_profiles: RunSpec[];
}
export type TrustLevel = "bundled" | "remote_unsigned";

export interface CatalogEntry {
  manifest: ToolManifest;
  trust: TrustLevel;
}

/** Backend install lifecycle (mirror of Rust `InstallState`). */
export type InstallState =
  | "not_installed"
  | "queued"
  | "downloading"
  | "installing"
  | "verifying"
  | "installed"
  | "update_available"
  | "uninstalling"
  | "cancelling"
  | "cancelled"
  | "failed"
  | "verification_failed";

/** `state` is null when the backend cannot determine it. */
export interface ToolStateEntry {
  tool_id: string;
  state: InstallState | null;
}

export type VerificationOutcome = "passed" | "failed" | "unavailable";
export type VerificationLevel = "package_record" | "executable" | "none";

export interface VerificationResult {
  at: string;
  outcome: VerificationOutcome;
  level: VerificationLevel;
  detail: string;
}

export interface InstalledTool {
  tool_id: string;
  version?: string | null;
  location?: string | null;
  runtime: RuntimeKind;
  state: InstallState;
  last_verification?: VerificationResult | null;
}

export type JobKind = "install" | "update" | "uninstall" | "verify" | "run" | "setup";
export type JobPhase =
  | "queued"
  | "checking_prerequisites"
  | "awaiting_consent"
  | "installing"
  | "verifying"
  | "uninstalling"
  | "running"
  | "cancelling"
  | "rolling_back"
  | "succeeded"
  | "failed"
  | "cancelled";

export interface JobError {
  code: string;
  message: string;
  recovery: string;
  detail?: string | null;
}

export interface Job {
  id: string;
  tool_id: string;
  kind: JobKind;
  phase: JobPhase;
  message: string;
  percent?: number | null;
  started_at: string;
  updated_at: string;
  finished_at?: string | null;
  exit_code?: number | null;
  allow_elevation: boolean;
  error?: JobError | null;
}

export interface Capabilities {
  install: boolean;
  uninstall: boolean;
  verify: boolean;
  launch: boolean;
  jobs: boolean;
  labs: boolean;
  installed_detection: boolean;
  /** Install methods that have a working adapter in this build. */
  install_methods: InstallMethod[];
}

export interface PrerequisiteStatus {
  spec: PrerequisiteSpec;
  /** null = could not be checked */
  satisfied: boolean | null;
}

export interface InstallPlan {
  tool_id: string;
  method: InstallSpec;
  prerequisites: PrerequisiteStatus[];
  missing_prerequisites: Prerequisite[];
  dependencies: string[];
  elevation: ElevationNeed;
  may_require_restart: boolean;
  network_required: boolean;
  approx_download_mb?: number | null;
  approx_disk_mb?: number | null;
  license: string;
  integrity: IntegrityLevel;
  notes: string[];
  unsupported_reason?: string | null;
}

export type LinkKind = "homepage" | "documentation" | "source_repository";

export type ComponentStatus = { component: Prerequisite } & (
  | { state: "available"; version?: string | null }
  | { state: "missing" }
  | { state: "unknown"; reason: string }
);

export interface SetupIssue {
  severity: "info" | "warning" | "blocking";
  title: string;
  recovery: string;
}

export interface EnvironmentReport {
  os: string;
  supported_os: boolean;
  windows_build?: number | null;
  architecture: string;
  free_disk_bytes?: number | null;
  components: ComponentStatus[];
  issues: SetupIssue[];
}

export interface ActivityEvent {
  at: string;
  kind: string;
  tool_id?: string | null;
  summary: string;
}

export interface AppInfo {
  name: string;
  version: string;
  data_dir: string;
  rejected_manifests: string[];
}

export interface ToolQuery {
  text?: string | null;
  category?: Category | null;
}

/** Error shape returned by every failing backend command. */
export interface AppErrorDto {
  code: string;
  message: string;
  recovery: string;
}



// ---------------------------------------------------------------- platform report (Phase 4)

export type Readiness = "ready" | "needs_setup" | "blocked" | "unknown";
export type FeatureState = "available" | "unavailable" | "unknown";
export type DistroState = "running" | "stopped" | "installing" | "unknown";

export interface WslDistro {
  name: string;
  state: DistroState;
  version?: number | null;
  is_default: boolean;
  internal: boolean;
  os_id?: string | null;
}

export type DaemonState =
  | { state: "running"; server_version?: string | null; os_type?: string | null; architecture?: string | null; rootless?: boolean | null }
  | { state: "stopped"; reason: string }
  | { state: "unknown"; reason: string };

export interface CapabilityReadiness {
  state: Readiness;
  summary: string;
  missing: string[];
  blockers: string[];
  notes: string[];
}

export type SetupActionId =
  | "install_wsl"
  | "install_distro"
  | "convert_distro_to_wsl2"
  | "enable_virtualization"
  | "install_docker_desktop"
  | "start_docker_desktop"
  | "free_disk_space"
  | "install_winget";

export interface SetupAction {
  id: SetupActionId;
  title: string;
  why: string;
  kind: "automatic" | "manual";
  requires_admin: boolean;
  requires_restart: boolean;
  command_preview?: string | null;
  official_link?: string | null;
  instructions: string[];
  depends_on: SetupActionId[];
}

export interface PlatformReport {
  detected_at: string;
  windows: { build?: number | null; display_version?: string | null; edition?: string | null; architecture: string };
  virtualization: { firmware_enabled?: boolean | null; hypervisor_present?: boolean | null };
  features: { id: string; label: string; state: FeatureState; basis: string }[];
  wsl: { installed?: boolean | null; detail?: string | null; wsl_version?: string | null; default_version?: number | null; distros: WslDistro[] };
  containers: {
    runtime?: {
      name: string;
      cli_path?: string | null;
      cli_version?: string | null;
      context?: string | null;
      context_local?: boolean | null;
      daemon: DaemonState;
    } | null;
  };
  package_managers: { name: string; scope: string; version?: string | null }[];
  disk: { path: string; free_bytes?: number | null; total_bytes?: number | null };
  system: { pending_restart?: boolean | null; proxy?: string | null; policy_keys: string[]; is_elevated?: boolean | null };
  configured_distro?: string | null;
  readiness: { windows_tools: CapabilityReadiness; linux_tools_wsl: CapabilityReadiness; containers: CapabilityReadiness };
  setup_actions: SetupAction[];
}

// ---------------------------------------------------------------- container launch

export interface ScopeAck {
  acknowledged: boolean;
  typed?: string | null;
  /** The addresses the user saw when confirming. They must still match at run time. */
  addresses: string[];
}

export interface RunApprovals {
  network: boolean;
  /** mount name -> host folder */
  mounts: Record<string, string>;
  /** container port -> host port */
  ports: Record<number, string | number>;
  /** Confirmation for targets outside the lab. */
  scope?: ScopeAck | null;
}

export interface RunSummary {
  image: string;
  command: string;
  user: string;
  network: "none" | "bridge";
  mounts: { host: string; container: string; read_only: boolean }[];
  ports: [number, number][];
  limits: string;
  read_only_rootfs: boolean;
  warnings: string[];
}

export type Locality = "loopback" | "lab" | "private_network" | "public" | "unspecified" | "special" | "unresolvable";
export type Verdict = "allowed" | "needs_confirmation" | "blocked";

export interface TargetAssessment {
  parameter: string;
  raw: string;
  canonical: string;
  form: "host" | "ip" | "url";
  addresses: string[];
  locality: Locality;
  verdict: Verdict;
  reasons: string[];
}

export interface ScopeReport {
  targets: TargetAssessment[];
  verdict: Verdict;
  confirmation?: { level: "acknowledge" | "type_target"; phrase?: string | null } | null;
  blocked_reasons: string[];
  redirects: { from: string; to: string; locality: Locality }[];
  lab_targets: string[];
}

export interface RunPreview {
  command: { runtime: RuntimeKind; program: string; args: string[]; display: string };
  container?: RunSummary | null;
  scope?: ScopeReport | null;
  network_activity: NetworkActivity;
  timeout_secs: number;
  output_format: OutputFormat;
  expected_output: string;
  working_dir: string;
  notes: string[];
}

export type RunOutcome = "clean" | "findings" | "failed" | "timed_out" | "cancelled";

export interface RunRecord {
  id: string;
  tool_id: string;
  tool_name: string;
  profile_id: string;
  profile_label: string;
  tool_version?: string | null;
  started_at: string;
  finished_at?: string | null;
  duration_ms?: number | null;
  exit_code?: number | null;
  /** null while the run is still going */
  outcome?: RunOutcome | null;
  command: string;
  targets: string[];
  scope: string;
  network_activity: NetworkActivity;
  output_format: OutputFormat;
  run_dir?: string | null;
  result_path?: string | null;
}

export interface OutputRow {
  seq: number;
  stream: "stdout" | "stderr";
  text: string;
}

export interface LaunchDefaults {
  parameters: Record<string, ParamValue>;
  mounts: Record<string, string>;
}

export interface UserSettings {
  scope: { lab_targets: string[] };
  retention: { history_days: number; output_days: number };
}

export interface PurgeReport {
  runs_deleted: number;
  output_lines_deleted: number;
  jobs_deleted: number;
}

// ---------------------------------------------------------------- learning

export type LabKind = "web_app" | "sample_project";
export type LabState = "stopped" | "starting" | "running" | "stopping" | "failed";

export interface LabDefinition {
  id: string;
  name: string;
  description: string;
  kind: LabKind;
  address?: string | null;
  folder?: string | null;
  weaknesses: string[];
  warning: string;
}

export type LabStatus = LabDefinition & { state: LabState };

export interface Tutorial {
  tool_id: string;
  title: string;
  what_it_does: string;
  problem_solved: string;
  requirements: string[];
  authorization_warning: string;
  first_run: { title: string; body: string; profile?: string | null }[];
  parameters: { profile: string; name: string; explanation: string }[];
  example: { profile: string; output: string; interpretation: string[] };
  common_errors: { symptom: string; cause: string; fix: string }[];
  exercise: { title: string; lab: string; profile: string; goal: string; steps: string[]; expected: string; stretch?: string | null };
  links: { label: string; url: string }[];
}

