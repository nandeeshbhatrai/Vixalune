import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { AppRoutes } from "@/App";
import type { Backend } from "@/domain/backend";
import type { Capabilities } from "@/domain/types";
import { createDemoBackend } from "@/infra/demoBackend";
import { BackendProvider } from "@/infra/backendContext";

export const NO_CAPS: Capabilities = {
  install: false,
  uninstall: false,
  verify: false,
  launch: false,
  jobs: false,
  labs: false,
  installed_detection: false,
  install_methods: [],
};

/** A non-demo backend that behaves like the Phase 2 core: catalog and environment only. */
export function readOnlyBackend(overrides: Partial<Backend> = {}): Backend {
  return {
    ...createDemoBackend(),
    isDemo: false,
    getCapabilities: async () => NO_CAPS,
    listToolStates: async () => [],
    listInstalledTools: async () => [],
    listJobs: async () => [],
    listLabs: async () => [],
    recentActivity: async () => [],
    ...overrides,
  };
}

export function renderApp(path: string, backend: Backend = createDemoBackend()) {
  return render(
    <BackendProvider backend={backend}>
      <MemoryRouter initialEntries={[path]}>
        <AppRoutes />
      </MemoryRouter>
    </BackendProvider>,
  );
}

