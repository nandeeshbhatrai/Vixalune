import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { deriveStatus, missingPrerequisites, type ToolStatus } from "@/domain/toolStatus";
import type { AppErrorDto, Capabilities, CatalogEntry, EnvironmentReport, InstallState, Job, Prerequisite } from "@/domain/types";
import { useBackend } from "@/infra/backendContext";
import { isActivePhase } from "@/presentation/labels";
import { toAppError, useAsync, type AsyncState } from "./useAsync";

interface CoreData {
  caps: Capabilities;
  tools: CatalogEntry[];
  states: Map<string, InstallState | null>;
}

export interface Workspace {
  core: AsyncState<CoreData>;
  /** Job list, refreshed while any job is active. */
  jobs: AsyncState<Job[]>;
  /** null while loading or when detection failed; see `envError`. */
  env: EnvironmentReport | null;
  envLoading: boolean;
  envError: AppErrorDto | null;
  /** Reload catalog, states and jobs without flashing loading placeholders. */
  refreshAll: () => void;
  reloadJobs: () => void;
  recheckEnvironment: () => void;
  statusOf: (toolId: string) => ToolStatus;
  missingFor: (toolId: string) => Prerequisite[] | null;
  activeJobFor: (toolId: string) => Job | undefined;
}

const WorkspaceContext = createContext<Workspace | null>(null);
const POLL_MS = 1500;

/** Loads catalog, capabilities, tool states, jobs and environment once and shares them across screens. */
export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const backend = useBackend();
  const [core, setCore] = useState<AsyncState<CoreData>>({ status: "loading" });
  const [jobs, setJobs] = useState<AsyncState<Job[]>>({ status: "loading" });
  const [envState, reloadEnv] = useAsync(() => backend.detectEnvironment(), [backend]);

  const loadCore = useCallback(async () => {
    try {
      const [caps, tools, stateList] = await Promise.all([backend.getCapabilities(), backend.listTools(), backend.listToolStates()]);
      setCore({ status: "ready", data: { caps, tools, states: new Map(stateList.map((s) => [s.tool_id, s.state])) } });
    } catch (e) {
      setCore((prev) => (prev.status === "ready" ? prev : { status: "error", error: toAppError(e) }));
    }
  }, [backend]);

  const loadJobs = useCallback(async () => {
    try {
      setJobs({ status: "ready", data: await backend.listJobs() });
    } catch (e) {
      setJobs((prev) => (prev.status === "ready" ? prev : { status: "error", error: toAppError(e) }));
    }
  }, [backend]);

  // Initial load, then ask the backend to re-detect installed tools and reload once it is done.
  useEffect(() => {
    let live = true;
    void loadCore().then(async () => {
      try {
        await backend.refreshToolStates();
        if (live) await loadCore();
      } catch {
        /* detection is best-effort; unknown states stay unknown */
      }
    });
    void loadJobs();
    return () => {
      live = false;
    };
  }, [backend, loadCore, loadJobs]);

  // Poll while something is running, and refresh tool states when the last active job finishes.
  const anyActive = jobs.status === "ready" && jobs.data.some((j) => isActivePhase(j.phase));
  useEffect(() => {
    if (!anyActive) return;
    const t = setInterval(() => void loadJobs(), POLL_MS);
    return () => clearInterval(t);
  }, [anyActive, loadJobs]);
  const wasActive = useRef(false);
  useEffect(() => {
    if (wasActive.current && !anyActive) void loadCore();
    wasActive.current = anyActive;
  }, [anyActive, loadCore]);

  const env = envState.status === "ready" ? envState.data : null;
  const coreData = core.status === "ready" ? core.data : null;

  const missingFor = useCallback(
    (id: string) => {
      const entry = coreData?.tools.find((t) => t.manifest.id === id);
      return entry ? missingPrerequisites(entry.manifest, env) : null;
    },
    [coreData, env],
  );
  const statusOf = useCallback(
    (id: string): ToolStatus => deriveStatus(coreData?.states.get(id) ?? null, missingFor(id)),
    [coreData, missingFor],
  );
  const activeJobFor = useCallback(
    (id: string) => (jobs.status === "ready" ? jobs.data.find((j) => j.tool_id === id && isActivePhase(j.phase)) : undefined),
    [jobs],
  );

  const value = useMemo<Workspace>(
    () => ({
      core,
      jobs,
      env,
      envLoading: envState.status === "loading",
      envError: envState.status === "error" ? envState.error : null,
      refreshAll: () => {
        void loadCore();
        void loadJobs();
      },
      reloadJobs: () => void loadJobs(),
      recheckEnvironment: reloadEnv,
      statusOf,
      missingFor,
      activeJobFor,
    }),
    [core, jobs, env, envState, loadCore, loadJobs, reloadEnv, statusOf, missingFor, activeJobFor],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace(): Workspace {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace must be used inside <WorkspaceProvider>");
  return ctx;
}

export { toAppError };
