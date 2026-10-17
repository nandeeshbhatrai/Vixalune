import { invoke } from "@tauri-apps/api/core";
import type { Backend } from "@/domain/backend";
import type { ActivityEvent, AppInfo, CatalogEntry, EnvironmentReport, ToolQuery } from "@/domain/types";

export const tauriBackend: Backend = {
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  detectEnvironment: () => invoke<EnvironmentReport>("detect_environment"),
  listTools: (query?: ToolQuery) => invoke<CatalogEntry[]>("list_tools", { query: query ?? null }),
  getTool: (id: string) => invoke<CatalogEntry>("get_tool", { id }),
  recentActivity: (limit?: number) => invoke<ActivityEvent[]>("recent_activity", { limit: limit ?? null }),
};
