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
  | { method: "winget"; package_id: string; version?: string | null }
  | { method: "wsl_apt"; packages: string[] }
  | { method: "container_image"; image: string; digest: string }
  | { method: "download_archive"; url: string; sha256: string; executable: string };

export type InstallMethod = InstallSpec["method"];

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

export interface ToolManifest {
  schema_version: number;
  id: string;
  name: string;
  summary: string;
  description: string;
  category: Category;
  license: string;
  homepage: string;
  documentation: string;
  source_repository?: string | null;
  maintenance?: MaintenanceStatus | null;
  recommended_for_beginners: boolean;
  platforms: RuntimeKind[];
  prerequisites: Prerequisite[];
  install: InstallSpec[];
  requires_elevation: boolean;
  verify: {
    runtime: RuntimeKind;
    executable: string;
    args: string[];
    expect_output_contains?: string | null;
  };
  run?: { runtime: RuntimeKind; executable: string; parameters: ParameterDef[] } | null;
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

export interface VerificationResult {
  at: string;
  passed: boolean;
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

export type JobKind = "install" | "uninstall" | "verify" | "run";
export type JobState = "queued" | "running" | "succeeded" | "failed" | "cancelled";

export interface Job {
  id: string;
  tool_id: string;
  kind: JobKind;
  state: JobState;
  started_at: string;
  finished_at?: string | null;
  exit_code?: number | null;
}

export interface Capabilities {
  install: boolean;
  uninstall: boolean;
  verify: boolean;
  launch: boolean;
  jobs: boolean;
  labs: boolean;
  installed_detection: boolean;
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
