import type {
  ActivityEvent,
  AppInfo,
  Capabilities,
  CatalogEntry,
  EnvironmentReport,
  InstalledTool,
  InstallPlan,
  Job,
  LabDefinition,
  LinkKind,
  ToolQuery,
  ToolStateEntry,
} from "./types";

/**
 * The UI's only view of the backend. Implemented by the Tauri adapter, by the explicitly
 * flagged demo backend, and by `unavailableBackend` when neither exists. Components must not
 * call `invoke`, run shell commands or touch the filesystem.
 */
export interface Backend {
  /** True only for the demonstration backend; the UI labels all of its data as demo data. */
  readonly isDemo: boolean;

  // Read models
  getAppInfo(): Promise<AppInfo>;
  getCapabilities(): Promise<Capabilities>;
  detectEnvironment(): Promise<EnvironmentReport>;
  listTools(query?: ToolQuery): Promise<CatalogEntry[]>;
  getTool(id: string): Promise<CatalogEntry>;
  listToolStates(): Promise<ToolStateEntry[]>;
  /** Ask the backend to re-detect which tools are present, then persist the answer. */
  refreshToolStates(): Promise<void>;
  listInstalledTools(): Promise<InstalledTool[]>;
  listJobs(): Promise<Job[]>;
  getJobOutput(jobId: string): Promise<string[]>;
  recentActivity(limit?: number): Promise<ActivityEvent[]>;
  listLabs(): Promise<LabDefinition[]>;
  /** What installing would do on this machine: prerequisites, privileges, size, blockers. */
  getInstallPlan(toolId: string): Promise<InstallPlan>;

  // Actions. Each resolves with a job id (or rejects with an AppErrorDto) and never blocks on the work itself.
  installTool(id: string, allowElevation: boolean): Promise<string>;
  uninstallTool(id: string, allowElevation: boolean): Promise<string>;
  verifyTool(id: string): Promise<string>;
  launchTool(id: string): Promise<string>;
  /** Grant the consent a job is waiting for. */
  approveJob(jobId: string): Promise<void>;
  cancelJob(jobId: string): Promise<void>;
  /** Opens a catalog link in the default browser. The backend resolves the URL. */
  openLink(toolId: string, kind: LinkKind): Promise<void>;
}
