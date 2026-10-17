import type { Backend } from "@/domain/backend";
import type {
  CatalogEntry,
  InstalledTool,
  InstallState,
  Job,
  LabDefinition,
  ToolManifest,
} from "@/domain/types";

/**
 * DEMONSTRATION DATA ONLY. Nothing here reflects the real machine. This backend is selected
 * only when `VITE_DEMO_DATA=true` (see `npm run dev:demo`) or injected by tests, and the UI
 * shows a persistent "Demonstration data" banner while it is active.
 */

type ToolSeed = Pick<ToolManifest, "id" | "name" | "summary" | "category"> & Partial<ToolManifest>;

function tool(seed: ToolSeed): CatalogEntry {
  return {
    trust: "bundled",
    manifest: {
      schema_version: 1,
      description: seed.summary,
      license: "Apache-2.0",
      homepage: `https://example.invalid/${seed.id}`,
      documentation: `https://example.invalid/${seed.id}/docs`,
      source_repository: `https://example.invalid/${seed.id}/source`,
      maintenance: "active",
      recommended_for_beginners: false,
      platforms: ["windows_native"],
      prerequisites: ["winget"],
      install: [{ method: "winget", package_id: "Vendor.Tool" }],
      requires_elevation: false,
      verify: { runtime: "windows_native", executable: seed.id, args: ["--version"] },
      run: null,
      ...seed,
    },
  };
}

export const DEMO_TOOLS: CatalogEntry[] = [
  tool({
    id: "nmap",
    name: "Nmap",
    summary: "Network discovery and port scanner.",
    description: "Discovers hosts and services on a network. Demo description.",
    category: "reconnaissance",
    license: "LicenseRef-Nmap",
    recommended_for_beginners: true,
    install: [{ method: "winget", package_id: "Insecure.Nmap" }],
  }),
  tool({
    id: "zap",
    name: "OWASP ZAP",
    summary: "Intercepting proxy and scanner for web application testing.",
    category: "web_application",
    recommended_for_beginners: true,
    install: [{ method: "winget", package_id: "ZAP.ZAP" }],
  }),
  tool({
    id: "wireshark",
    name: "Wireshark",
    summary: "Network protocol analyser.",
    category: "network",
    license: "GPL-2.0-or-later",
    recommended_for_beginners: true,
    requires_elevation: true,
    install: [{ method: "winget", package_id: "WiresharkFoundation.Wireshark" }],
  }),
  tool({
    id: "semgrep",
    name: "Semgrep",
    summary: "Fast static analysis for finding bugs and insecure code patterns.",
    category: "vulnerability",
    license: "LGPL-2.1-only",
    platforms: ["container"],
    prerequisites: ["docker"],
    install: [{ method: "container_image", image: "semgrep/semgrep", digest: `sha256:${"0".repeat(64)}` }],
  }),
  tool({
    id: "trivy",
    name: "Trivy",
    summary: "Vulnerability and misconfiguration scanner.",
    category: "vulnerability",
    install: [{ method: "winget", package_id: "AquaSecurity.Trivy" }],
  }),
];

const DEMO_LABS: LabDefinition[] = [
  {
    id: "demo-web-lab",
    name: "Demo: intentionally vulnerable web app",
    description: "Placeholder lab entry shown only in demo mode.",
    image: "example/demo-lab",
    digest: `sha256:${"0".repeat(64)}`,
    ports: [{ container: 80, host: 8080 }],
  },
];

function minutesAgo(m: number): string {
  return new Date(Date.now() - m * 60_000).toISOString();
}

export function createDemoBackend(): Backend {
  const states = new Map<string, InstallState>([
    ["nmap", "installed"],
    ["wireshark", "update_available"],
    ["zap", "verification_failed"],
    ["trivy", "not_installed"],
    ["semgrep", "not_installed"],
  ]);

  const installed: InstalledTool[] = [
    {
      tool_id: "nmap",
      version: "7.95 (demo)",
      location: "C:\\Demo\\Tools\\nmap",
      runtime: "windows_native",
      state: "installed",
      last_verification: { at: minutesAgo(30), passed: true, detail: "Demo: nmap --version succeeded" },
    },
    {
      tool_id: "wireshark",
      version: "4.2.0 (demo)",
      location: "C:\\Demo\\Tools\\wireshark",
      runtime: "windows_native",
      state: "update_available",
      last_verification: { at: minutesAgo(90), passed: true, detail: "Demo: tshark --version succeeded" },
    },
    {
      tool_id: "zap",
      version: "2.15.0 (demo)",
      location: "C:\\Demo\\Tools\\zap",
      runtime: "windows_native",
      state: "verification_failed",
      last_verification: { at: minutesAgo(10), passed: false, detail: "Demo: expected output was not found" },
    },
  ];

  const jobs: Job[] = [
    { id: "demo-job-1", tool_id: "nmap", kind: "install", state: "succeeded", started_at: minutesAgo(32), finished_at: minutesAgo(31), exit_code: 0 },
    { id: "demo-job-2", tool_id: "zap", kind: "verify", state: "failed", started_at: minutesAgo(10), finished_at: minutesAgo(10), exit_code: 1 },
    { id: "demo-job-3", tool_id: "wireshark", kind: "verify", state: "running", started_at: minutesAgo(1), finished_at: null, exit_code: null },
  ];
  const output: Record<string, string[]> = {
    "demo-job-1": ["[demo] Installing nmap…", "[demo] Done."],
    "demo-job-2": ["[demo] Running verification…", "[demo] Expected text was not found."],
    "demo-job-3": ["[demo] Running verification…"],
  };

  const notFound = (id: string) => ({ code: "not_found", message: `tool '${id}' was not found`, recovery: "Refresh the catalog and try again." });
  const need = (id: string) => {
    if (!DEMO_TOOLS.some((t) => t.manifest.id === id)) return Promise.reject(notFound(id));
    return null;
  };

  return {
    isDemo: true,
    getAppInfo: async () => ({ name: "Vixalune", version: "demo", data_dir: "(demo)", rejected_manifests: [] }),
    getCapabilities: async () => ({ install: true, uninstall: true, verify: true, launch: true, jobs: true, labs: false, installed_detection: true }),
    detectEnvironment: async () => ({
      os: "windows",
      supported_os: true,
      components: [
        { component: "winget", state: "available", version: "v1.9 (demo)" },
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
      return DEMO_TOOLS.filter(
        (t) =>
          (!query?.category || t.manifest.category === query.category) &&
          (!text || t.manifest.name.toLowerCase().includes(text) || t.manifest.summary.toLowerCase().includes(text)),
      );
    },
    getTool: async (id) => DEMO_TOOLS.find((t) => t.manifest.id === id) ?? Promise.reject(notFound(id)),
    listToolStates: async () => [...states].map(([tool_id, state]) => ({ tool_id, state })),
    listInstalledTools: async () => installed.filter((i) => states.get(i.tool_id) !== "not_installed"),
    listJobs: async () => [...jobs],
    getJobOutput: async (jobId) => output[jobId] ?? [],
    recentActivity: async () => [
      { at: minutesAgo(1), kind: "app_started", summary: "Demo: Vixalune started" },
      { at: minutesAgo(10), kind: "install_failed", tool_id: "zap", summary: "Demo: verification of ZAP failed" },
      { at: minutesAgo(31), kind: "install_succeeded", tool_id: "nmap", summary: "Demo: installed Nmap" },
    ],
    listLabs: async () => DEMO_LABS,
    installTool: async (id) => {
      const bad = need(id);
      if (bad) return bad;
      states.set(id, "installing");
      return "demo-op";
    },
    uninstallTool: async (id) => {
      const bad = need(id);
      if (bad) return bad;
      states.set(id, "not_installed");
      return "demo-op";
    },
    verifyTool: async (id) => {
      const bad = need(id);
      if (bad) return bad;
      states.set(id, "installed");
      return "installed";
    },
    launchTool: async (id) => {
      const bad = need(id);
      if (bad) return bad;
      return "demo-run";
    },
    cancelJob: async (jobId) => {
      const job = jobs.find((j) => j.id === jobId);
      if (job) {
        job.state = "cancelled";
        job.finished_at = new Date().toISOString();
      }
    },
    openLink: async () => {
      // Demo URLs are placeholders; nothing is opened.
    },
  };
}
