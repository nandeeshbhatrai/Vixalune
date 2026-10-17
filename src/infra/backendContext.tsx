import { createContext, useContext, type ReactNode } from "react";
import type { Backend } from "@/domain/backend";
import { createDemoBackend } from "./demoBackend";
import { tauriBackend } from "./tauriBackend";
import { unavailableBackend } from "./unavailableBackend";

/** Tauri when running in the desktop shell; demo data only behind an explicit flag; else unavailable. */
export function selectBackend(): Backend {
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) return tauriBackend;
  if (import.meta.env.VITE_DEMO_DATA === "true") return createDemoBackend();
  return unavailableBackend;
}

const BackendContext = createContext<Backend>(unavailableBackend);

export function BackendProvider({ backend, children }: { backend: Backend; children: ReactNode }) {
  return <BackendContext.Provider value={backend}>{children}</BackendContext.Provider>;
}

export function useBackend(): Backend {
  return useContext(BackendContext);
}
