import { invoke } from "@tauri-apps/api/core";
import type { Backend } from "@/domain/backend";
import type {
  ActivityEvent,
  AppInfo,
  Capabilities,
  CatalogEntry,
  EnvironmentReport,
  InstalledTool,
  InstallState,
  Job,
  LabDefinition,
  ToolQuery,
  ToolStateEntry,
} from "@/domain/types";

/** Thin typed wrapper over Tauri IPC. No logic lives here. */
export const tauriBackend: Backend = {
  isDemo: false,
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  getCapabilities: () => invoke<Capabilities>("get_capabilities"),
  detectEnvironment: () => invoke<EnvironmentReport>("detect_environment"),
  listTools: (query?: ToolQuery) => invoke<CatalogEntry[]>("list_tools", { query: query ?? null }),
  getTool: (id) => invoke<CatalogEntry>("get_tool", { id }),
  listToolStates: () => invoke<ToolStateEntry[]>("list_tool_states"),
  listInstalledTools: () => invoke<InstalledTool[]>("list_installed_tools"),
  listJobs: () => invoke<Job[]>("list_jobs"),
  getJobOutput: (jobId) => invoke<string[]>("get_job_output", { id: jobId }),
  recentActivity: (limit) => invoke<ActivityEvent[]>("recent_activity", { limit: limit ?? null }),
  listLabs: () => invoke<LabDefinition[]>("list_labs"),
  installTool: (id, allowElevation) => invoke<string>("install_tool", { id, allowElevation }),
  uninstallTool: (id, allowElevation) => invoke<string>("uninstall_tool", { id, allowElevation }),
  verifyTool: (id) => invoke<InstallState>("verify_tool", { id }),
  launchTool: (id) => invoke<string>("launch_tool", { id }),
  cancelJob: (jobId) => invoke<void>("cancel_job", { id: jobId }),
  openLink: (toolId, kind) => invoke<void>("open_link", { toolId, kind }),
};
