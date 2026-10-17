import type { Backend } from "@/domain/backend";
import type {
  CatalogEntry,
  InstalledTool,
  InstallPlan,
  InstallState,
  Job,
  JobPhase,
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
      schema_version: 2,
      manifest_version: "1.0.0",
      maintainer: "Demo",
      description: seed.summary,
      tags: [],
      license: "Apache-2.0",
      homepage: `https://example.invalid/${seed.id}`,
      documentation: `https://example.invalid/${seed.id}/docs`,
      source_repository: `https://example.invalid/${seed.id}/source`,
      provenance: { publisher: "Demo publisher", basis: "package_repository", source_url: "https://example.invalid", verified_on: "2026-10-17" },
      maintenance: "active",
      recommended_for_beginners: false,
      windows: { min_build: 19045, architectures: ["x64"] },
      platforms: ["windows_native"],
      prerequisites: [{ kind: "winget", reason: "Installs the tool from the Windows Package Manager.", requires_admin: false, requires_restart: false }],
      dependencies: [],
      install: [{ method: "winget", package_id: "Vendor.Tool" }],
      version: {},
      storage: {},
      integrity: { level: "package_manager", note: "winget checks the installer hash." },
      network: { install: true, run: false },
      privileges: { install: "none", run: "none", may_require_restart: false },
      lifecycle: { can_update: true, can_uninstall: true },
      verify: { checks: [{ kind: "package_installed" }] },
      launch_profiles: [],
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
    tags: ["scanner", "ports"],
    license: "LicenseRef-Nmap",
    recommended_for_beginners: true,
    privileges: { install: "required", run: "none", may_require_restart: false },
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
    privileges: { install: "required", run: "required", may_require_restart: false },
    install: [{ method: "winget", package_id: "WiresharkFoundation.Wireshark" }],
  }),
  tool({
    id: "semgrep",
    name: "Semgrep",
    summary: "Fast static analysis for finding bugs and insecure code patterns.",
    category: "vulnerability",
    license: "LGPL-2.1-only",
    platforms: ["container"],
    prerequisites: [{ kind: "docker", reason: "Semgrep runs as a container image.", requires_admin: false, requires_restart: false }],
    integrity: { level: "image_digest", note: "Pinned by digest." },
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

function job(partial: Pick<Job, "id" | "tool_id" | "kind" | "phase"> & Partial<Job>): Job {
  return { message: "", started_at: minutesAgo(1), updated_at: minutesAgo(1), allow_elevation: false, ...partial };
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
      last_verification: { at: minutesAgo(30), outcome: "passed", level: "executable", detail: "Demo: nmap.exe ran successfully" },
    },
    {
      tool_id: "wireshark",
      version: "4.2.0 (demo)",
      location: "C:\\Demo\\Tools\\wireshark",
      runtime: "windows_native",
      state: "update_available",
      last_verification: { at: minutesAgo(90), outcome: "passed", level: "package_record", detail: "Demo: package record found" },
    },
    {
      tool_id: "zap",
      version: "2.15.0 (demo)",
      location: "C:\\Demo\\Tools\\zap",
      runtime: "windows_native",
      state: "verification_failed",
      last_verification: { at: minutesAgo(10), outcome: "failed", level: "none", detail: "Demo: expected output was not found" },
    },
  ];

  const jobs: Job[] = [
    job({ id: "demo-job-1", tool_id: "nmap", kind: "install", phase: "succeeded", message: "Installed and verified", started_at: minutesAgo(32), finished_at: minutesAgo(31), exit_code: 0 }),
    job({
      id: "demo-job-2",
      tool_id: "zap",
      kind: "verify",
      phase: "failed",
      message: "Demo: expected text was not found",
      started_at: minutesAgo(10),
      finished_at: minutesAgo(10),
      exit_code: 1,
      error: {
        code: "verification_failed",
        message: "The tool did not pass verification, so the install is not reported as successful.",
        recovery: "Use Verify to re-check, Retry to reinstall, or Uninstall to remove it.",
        detail: "Demo: expected output was not found",
      },
    }),
    job({ id: "demo-job-3", tool_id: "wireshark", kind: "verify", phase: "verifying", message: "Verifying", percent: null }),
  ];
  const output: Record<string, string[]> = {
    "demo-job-1": ["[demo] > winget install --id Insecure.Nmap --exact", "[demo] Done."],
    "demo-job-2": ["[demo] Running verification…", "[demo] Expected text was not found."],
    "demo-job-3": ["[demo] Running verification…"],
  };

  const notFound = (id: string) => ({ code: "not_found", message: `tool '${id}' was not found`, recovery: "Refresh the catalog and try again." });
  const need = (id: string) => (DEMO_TOOLS.some((t) => t.manifest.id === id) ? null : Promise.reject(notFound(id)));
  let counter = 100;

  /** Demo-only: pretends a job progresses so the UI can be explored. Real jobs never do this. */
  const simulate = (toolId: string, kind: Job["kind"], allowElevation: boolean, finalState: InstallState): string => {
    const id = `demo-job-${++counter}`;
    const j = job({ id, tool_id: toolId, kind, phase: "installing", message: "Demo: working…", percent: 20, allow_elevation: allowElevation });
    jobs.unshift(j);
    output[id] = ["[demo] This is simulated output."];
    states.set(toolId, kind === "uninstall" ? "uninstalling" : "installing");
    const steps: [number, () => void][] = [
      [700, () => Object.assign(j, { percent: 60 })],
      [1400, () => Object.assign(j, { phase: "verifying" satisfies JobPhase, message: "Demo: verifying", percent: null })],
      [
        2100,
        () => {
          Object.assign(j, { phase: "succeeded" satisfies JobPhase, message: "Demo: finished", finished_at: new Date().toISOString(), exit_code: 0 });
          states.set(toolId, finalState);
        },
      ],
    ];
    for (const [ms, fn] of steps) setTimeout(() => j.phase !== "cancelled" && fn(), ms);
    return id;
  };

  return {
    isDemo: true,
    getAppInfo: async () => ({ name: "Vixalune", version: "demo", data_dir: "(demo)", rejected_manifests: [] }),
    getCapabilities: async () => ({
      install: true,
      uninstall: true,
      verify: true,
      launch: true,
      jobs: true,
      labs: false,
      installed_detection: true,
      install_methods: ["winget"],
    }),
    detectEnvironment: async () => ({
      os: "windows",
      supported_os: true,
      windows_build: 26100,
      architecture: "x64",
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
    refreshToolStates: async () => undefined,
    listInstalledTools: async () => installed.filter((i) => states.get(i.tool_id) !== "not_installed"),
    listJobs: async () => jobs.map((j) => ({ ...j })),
    getJobOutput: async (jobId) => output[jobId] ?? [],
    recentActivity: async () => [
      { at: minutesAgo(1), kind: "app_started", summary: "Demo: Vixalune started" },
      { at: minutesAgo(10), kind: "install_failed", tool_id: "zap", summary: "Demo: verification of ZAP failed" },
      { at: minutesAgo(31), kind: "install_succeeded", tool_id: "nmap", summary: "Demo: installed Nmap" },
    ],
    listLabs: async () => DEMO_LABS,
    getInstallPlan: async (toolId): Promise<InstallPlan> => {
      const entry = DEMO_TOOLS.find((t) => t.manifest.id === toolId);
      if (!entry) return Promise.reject(notFound(toolId));
      const m = entry.manifest;
      const docker = m.prerequisites.some((p) => p.kind === "docker");
      return {
        tool_id: m.id,
        method: m.install[0]!,
        prerequisites: m.prerequisites.map((spec) => ({ spec, satisfied: spec.kind === "winget" })),
        missing_prerequisites: docker ? ["docker"] : [],
        dependencies: m.dependencies,
        elevation: m.privileges.install,
        may_require_restart: m.privileges.may_require_restart,
        network_required: m.network.install,
        approx_download_mb: m.storage.approx_download_mb,
        approx_disk_mb: m.storage.approx_disk_mb,
        license: m.license,
        integrity: m.integrity.level,
        unsupported_reason: docker ? "Demo: this tool needs the container installer, which is not available." : null,
      };
    },
    installTool: async (id, allowElevation) => need(id) ?? simulate(id, "install", allowElevation, "installed"),
    uninstallTool: async (id, allowElevation) => need(id) ?? simulate(id, "uninstall", allowElevation, "not_installed"),
    verifyTool: async (id) => need(id) ?? simulate(id, "verify", false, "installed"),
    launchTool: async (id) => need(id) ?? "demo-run",
    approveJob: async (jobId) => {
      const j = jobs.find((x) => x.id === jobId);
      if (j?.phase === "awaiting_consent") j.phase = "installing";
    },
    cancelJob: async (jobId) => {
      const j = jobs.find((x) => x.id === jobId);
      if (j) {
        j.phase = "cancelled";
        j.finished_at = new Date().toISOString();
        states.set(j.tool_id, "not_installed");
      }
    },
    openLink: async () => {
      // Demo URLs are placeholders; nothing is opened.
    },
  };
}
