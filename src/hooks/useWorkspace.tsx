import { createContext, useCallback, useContext, useMemo, type ReactNode } from "react";
import { deriveStatus, missingPrerequisites, type ToolStatus } from "@/domain/toolStatus";
import type { AppErrorDto, Capabilities, CatalogEntry, EnvironmentReport, InstallState, Prerequisite } from "@/domain/types";
import { useBackend } from "@/infra/backendContext";
import { toAppError, useAsync, type AsyncState } from "./useAsync";

interface CoreData {
  caps: Capabilities;
  tools: CatalogEntry[];
  states: Map<string, InstallState | null>;
}

export interface Workspace {
  core: AsyncState<CoreData>;
  /** null while loading or when detection failed; see `envError`. */
  env: EnvironmentReport | null;
  envLoading: boolean;
  envError: AppErrorDto | null;
  refreshAll: () => void;
  recheckEnvironment: () => void;
  statusOf: (toolId: string) => ToolStatus;
  missingFor: (toolId: string) => Prerequisite[] | null;
}

const WorkspaceContext = createContext<Workspace | null>(null);

/** Loads catalog, capabilities, tool states and environment once and shares them across screens. */
export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const backend = useBackend();
  const [core, reloadCore] = useAsync<CoreData>(async () => {
    const [caps, tools, stateList] = await Promise.all([backend.getCapabilities(), backend.listTools(), backend.listToolStates()]);
    return { caps, tools, states: new Map(stateList.map((s) => [s.tool_id, s.state])) };
  }, [backend]);
  const [envState, reloadEnv] = useAsync(() => backend.detectEnvironment(), [backend]);

  const env = envState.status === "ready" ? envState.data : null;
  const coreData = core.status === "ready" ? core.data : null;

  const statusOf = useCallback(
    (id: string): ToolStatus => {
      const entry = coreData?.tools.find((t) => t.manifest.id === id);
      const missing = entry ? missingPrerequisites(entry.manifest, env) : null;
      return deriveStatus(coreData?.states.get(id) ?? null, missing);
    },
    [coreData, env],
  );
  const missingFor = useCallback(
    (id: string) => {
      const entry = coreData?.tools.find((t) => t.manifest.id === id);
      return entry ? missingPrerequisites(entry.manifest, env) : null;
    },
    [coreData, env],
  );

  const value = useMemo<Workspace>(
    () => ({
      core,
      env,
      envLoading: envState.status === "loading",
      envError: envState.status === "error" ? envState.error : null,
      refreshAll: () => {
        reloadCore();
        reloadEnv();
      },
      recheckEnvironment: reloadEnv,
      statusOf,
      missingFor,
    }),
    [core, env, envState, reloadCore, reloadEnv, statusOf, missingFor],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace(): Workspace {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace must be used inside <WorkspaceProvider>");
  return ctx;
}

export { toAppError };
