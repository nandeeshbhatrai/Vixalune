import type { ActivityEvent, AppInfo, CatalogEntry, EnvironmentReport, ToolQuery } from "./types";

/** The UI's only view of the backend. Implemented by the Tauri adapter and by the mock. */
export interface Backend {
  getAppInfo(): Promise<AppInfo>;
  detectEnvironment(): Promise<EnvironmentReport>;
  listTools(query?: ToolQuery): Promise<CatalogEntry[]>;
  getTool(id: string): Promise<CatalogEntry>;
  recentActivity(limit?: number): Promise<ActivityEvent[]>;
}
