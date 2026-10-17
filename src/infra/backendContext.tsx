import { createContext, useContext, type ReactNode } from "react";
import type { Backend } from "@/domain/backend";
import { mockBackend } from "./mockBackend";
import { tauriBackend } from "./tauriBackend";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const BackendContext = createContext<Backend>(isTauri ? tauriBackend : mockBackend);

export function BackendProvider({ backend, children }: { backend: Backend; children: ReactNode }) {
  return <BackendContext.Provider value={backend}>{children}</BackendContext.Provider>;
}

export function useBackend(): Backend {
  return useContext(BackendContext);
}
