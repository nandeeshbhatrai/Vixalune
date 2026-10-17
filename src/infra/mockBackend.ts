import type { Backend } from "@/domain/backend";
import type { CatalogEntry, ToolManifest } from "@/domain/types";

function tool(partial: Partial<ToolManifest> & Pick<ToolManifest, "id" | "name" | "summary" | "category">): CatalogEntry {
  return {
    trust: "bundled",
    manifest: {
      schema_version: 1,
      description: partial.summary,
      license: "GPL-2.0-or-later",
      homepage: "https://example.com",
      documentation: "https://example.com/docs",
      platforms: ["windows_native"],
      prerequisites: ["winget"],
      install: [{ method: "winget", package_id: "Vendor.Tool" }],
      requires_elevation: false,
      verify: { runtime: "windows_native", executable: partial.id, args: ["--version"] },
      run: null,
      ...partial,
    },
  };
}

const TOOLS: CatalogEntry[] = [
  tool({ id: "nmap", name: "Nmap", summary: "Network discovery and port scanner.", category: "reconnaissance" }),
  tool({ id: "wireshark", name: "Wireshark", summary: "Network protocol analyser.", category: "network", requires_elevation: true }),
  tool({
    id: "nikto",
    name: "Nikto",
    summary: "Web server vulnerability scanner (runs in WSL).",
    category: "web_application",
    platforms: ["wsl"],
    prerequisites: ["wsl2"],
    install: [{ method: "wsl_apt", packages: ["nikto"] }],
  }),
];

/** In-browser stand-in used by `vite dev` outside Tauri, unit tests and Playwright. */
export const mockBackend: Backend = {
  getAppInfo: async () => ({ name: "Vixalune", version: "0.1.0", data_dir: "(mock)", rejected_manifests: [] }),
  detectEnvironment: async () => ({
    os: "windows",
    supported_os: true,
    components: [
      { component: "winget", state: "available", version: "v1.9" },
      { component: "wsl2", state: "missing" },
      { component: "docker", state: "missing" },
    ],
    issues: [
      { severity: "info", title: "WSL 2 was not found", recovery: "Run 'wsl --install' as administrator, restart, then re-check." },
      { severity: "info", title: "Docker was not found", recovery: "Install Docker Desktop and start it, then re-check." },
    ],
  }),
  listTools: async (query) => {
    const text = query?.text?.toLowerCase() ?? "";
    return TOOLS.filter(
      (t) =>
        (!query?.category || t.manifest.category === query.category) &&
        (!text || t.manifest.name.toLowerCase().includes(text) || t.manifest.summary.toLowerCase().includes(text)),
    );
  },
  getTool: async (id) => {
    const found = TOOLS.find((t) => t.manifest.id === id);
    if (!found) throw { code: "not_found", message: `tool '${id}' was not found`, recovery: "Refresh the catalog and try again." };
    return found;
  },
  recentActivity: async () => [{ at: "2026-01-01T00:00:00Z", kind: "app_started", summary: "Vixalune started" }],
};
