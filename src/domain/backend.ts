import type {
  ActivityEvent,
  AppInfo,
  Capabilities,
  CatalogEntry,
  InstalledTool,
  InstallPlan,
  Job,
  LabDefinition,
  LinkKind,
  ParamValue,
  PlatformReport,
  RunApprovals,
  RunPreview,
  SetupActionId,
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
  /** Facts, per-capability readiness and proposed setup steps for this computer. */
  getPlatformReport(): Promise<PlatformReport>;
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
  /** Describe exactly what a launch would do (container image, mounts, network, ports) without starting it. */
  previewLaunch(id: string, profile: string, values: Record<string, ParamValue>, approvals: RunApprovals): Promise<RunPreview>;
  /** Start a launch profile as a job. Container runs start only with exactly these approvals. */
  launchTool(id: string, profile: string, values: Record<string, ParamValue>, approvals: RunApprovals): Promise<string>;
  /** Run one automatic setup step. `confirmed` must be true because it changes the computer. */
  runSetupAction(id: SetupActionId, confirmed: boolean): Promise<string>;
  /** Schedule a Windows restart (cancellable delay). Never happens without `confirmed`. */
  requestRestart(confirmed: boolean, delaySecs: number): Promise<void>;
  cancelRestart(): Promise<void>;
  /** Open the official guide for a setup step in the default browser (the backend owns the URL). */
  openSetupLink(id: SetupActionId): Promise<void>;
  /** Grant the consent a job is waiting for. */
  approveJob(jobId: string): Promise<void>;
  cancelJob(jobId: string): Promise<void>;
  /** Opens a catalog link in the default browser. The backend resolves the URL. */
  openLink(toolId: string, kind: LinkKind): Promise<void>;
}


