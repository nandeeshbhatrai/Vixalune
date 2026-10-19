import type {
  ActivityEvent,
  AppInfo,
  Capabilities,
  CatalogEntry,
  InstalledTool,
  InstallPlan,
  Job,
  LabStatus,
  LaunchDefaults,
  OutputRow,
  PurgeReport,
  RunRecord,
  StorageInfo,
  StorageUpdate,
  Tutorial,
  UserSettings,
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
  listLabs(): Promise<LabStatus[]>;
  /** Start or stop a learning lab (the practice web application runs only on 127.0.0.1). Returns the updated list. */
  startLab(id: string): Promise<LabStatus[]>;
  stopLab(id: string): Promise<LabStatus[]>;
  /** Form defaults for a profile, with lab placeholders already replaced by real values. */
  getLaunchDefaults(toolId: string, profile: string): Promise<LaunchDefaults>;
  getJob(jobId: string): Promise<Job>;
  /** Output lines after sequence number `after` (0 = from the start). Poll this while a run is active. */
  getRunOutput(jobId: string, after: number): Promise<OutputRow[]>;
  listRuns(limit?: number): Promise<RunRecord[]>;
  getRun(jobId: string): Promise<RunRecord>;
  /** The backend opens the Save dialog and writes the stored output; resolves with the saved path, or null if cancelled. */
  saveRunOutput(jobId: string): Promise<string | null>;
  /** Show the run's own folder (report files) in Explorer. */
  openRunFolder(jobId: string): Promise<void>;
  getSettings(): Promise<UserSettings>;
  updateSettings(settings: UserSettings): Promise<UserSettings>;
  /** Delete all finished run history and stored output now. */
  purgeHistoryNow(): Promise<PurgeReport>;
  listTutorials(): Promise<Tutorial[]>;
  getTutorial(toolId: string): Promise<Tutorial>;
  /** Open one of a tutorial's official links (the backend reads the address from the validated tutorial). */
  openTutorialLink(toolId: string, index: number): Promise<void>;
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
  /** Where data and tools are stored, and which locations can be changed. */
  getStorageInfo(): Promise<StorageInfo>;
  /** Change the tools and labs folders. They take effect after a restart; the backend validates and creates them. */
  updateStorage(update: StorageUpdate): Promise<StorageInfo>;
  /** Open the project's release page in the default browser (a fixed address owned by the backend). Vixalune never downloads or runs updates itself. */
  openReleasePage(): Promise<void>;
  /** Opens a catalog link in the default browser. The backend resolves the URL. */
  openLink(toolId: string, kind: LinkKind): Promise<void>;
}


