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

export type InstallSpec =
  | { method: "winget"; package_id: string; version?: string | null }
  | { method: "wsl_apt"; packages: string[] }
  | { method: "container_image"; image: string; digest: string }
  | { method: "download_archive"; url: string; sha256: string; executable: string };

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
  | "failed";

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
