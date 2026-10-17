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

export type ParamKind =
  | { type: "text"; max_len?: number | null }
  | { type: "host" }
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

export interface RunSpec {
  id: string;
  label: string;
  description: string;
  runtime: RuntimeKind;
  executable: string;
  parameters: ParameterDef[];
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

export type JobKind = "install" | "update" | "uninstall" | "verify" | "run";
export type JobPhase =
  | "queued"
  | "checking_prerequisites"
  | "awaiting_consent"
  | "installing"
  | "verifying"
  | "uninstalling"
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
  unsupported_reason?: string | null;
}
export interface LabDefinition {
  id: string;
  name: string;
  description: string;
  image: string;
  digest: string;
  ports: { container: number; host: number }[];
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

