import type { Backend } from "@/domain/backend";
import { createDemoRuns } from "@/infra/demoRuns";
import type {
  CapabilityReadiness,
  CatalogEntry,
  InstalledTool,
  InstallPlan,
  InstallState,
  Job,
  JobPhase,
  RunSpec,
  StorageInfo,
  PlatformReport,
  SetupAction,
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
      container: null,
      launch_profiles: [],
      ...seed,
    },
  };
}

function nativeProfile(id: string, label: string, description: string, executable: string, parameters: RunSpec["parameters"], expected: string, timeout: number, format: RunSpec["output_format"] = "text"): RunSpec {
  return {
    id,
    label,
    description,
    runtime: "windows_native",
    executable,
    parameters,
    mounts: [],
    needs_network: false,
    ports: [],
    working_dir: "run_dir",
    timeout_secs: timeout,
    output_format: format,
    network_activity: "probes_targets",
    exit_codes: { ok: [0], findings: [] },
    expected_output: expected,
    suggested_mounts: [],
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
    launch_profiles: [
      nativeProfile("quick-scan", "Scan the 100 most common ports", "A fast look at which common services a host exposes.", "nmap.exe", [
        { name: "target", label: "Target", description: "A host name or IP address you are authorised to scan.", kind: { type: "host" }, required: true, default: "127.0.0.1" },
      ], "A table of open ports (PORT STATE SERVICE).", 600),
      nativeProfile("service-scan", "Identify services on chosen ports", "Connects to the ports you list and tries to recognise the software answering.", "nmap.exe", [
        { name: "target", label: "Target", description: "A host name or IP address you are authorised to scan.", kind: { type: "host" }, required: true, default: "127.0.0.1" },
        { name: "ports", label: "Ports", description: "Ports to check, for example 80,443 or 8000-8100 (at most 32 entries).", kind: { type: "port_list", max_items: 32 }, required: true, default: "18080" },
      ], "For each open port, the service name and (when recognised) product and version.", 900),
    ],
  }),
  tool({
    id: "zap",
    name: "OWASP ZAP",
    summary: "Intercepting proxy and scanner for web application testing.",
    category: "web_application",
    recommended_for_beginners: true,
    install: [{ method: "winget", package_id: "ZAP.ZAP" }],
    launch_profiles: [
      nativeProfile("quick-scan", "Quick scan of a web application (spider + active scan)", "ZAP crawls the site and actively probes it. This sends many requests.", "zap.exe", [
        { name: "url", label: "Web application URL", description: "An http:// or https:// address.", kind: { type: "url" }, required: true, default: "http://127.0.0.1:18080/" },
      ], "Progress messages, then an HTML report in the run folder.", 1800, "html"),
    ],
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
    container: {
      user: "65532:65532",
      read_only_rootfs: true,
      memory_mb: 1024,
      cpus_milli: 1000,
      pids_limit: 256,
      tmpfs: ["/tmp"],
      workdir: "/src",
      env: [{ name: "HOME", value: "/tmp" }],
      mounts: [{ name: "source", container_path: "/src", read_only: true }],
      ports: [],
    },
    launch_profiles: [
      {
        id: "scan-local",
        label: "Scan a folder with your own rules (offline)",
        description: "No network access.",
        runtime: "container",
        executable: "semgrep",
        parameters: [{ name: "config", label: "Rules file inside the folder", description: "", kind: { type: "text", max_len: 200 }, required: true, default: "/src/.semgrep.yml" }],
        mounts: ["source"],
        needs_network: false,
        ports: [],
      },
      {
        id: "scan-sample",
        label: "Scan the practice code project (offline)",
        description: "Scans the safe sample project with the sample rules it contains. No network access.",
        runtime: "container",
        executable: "semgrep",
        parameters: [],
        mounts: ["source"],
        needs_network: false,
        ports: [],
        network_activity: "none",
        exit_codes: { ok: [0], findings: [1] },
        expected_output: "Several findings in sample files.",
        suggested_mounts: [{ name: "source", value: "C:\Demo\Vixalune\labs\sample-code" }],
      },
      {
        id: "scan-registry-rules",
        label: "Scan with the Semgrep registry rules (needs internet)",
        description: "Downloads the public rule set.",
        runtime: "container",
        executable: "semgrep",
        parameters: [],
        mounts: ["source"],
        needs_network: true,
        ports: [],
      },
    ],
  }),
  tool({
    id: "nikto",
    name: "Nikto",
    summary: "Web server vulnerability scanner (runs inside WSL).",
    category: "web_application",
    license: "GPL-2.0-or-later",
    platforms: ["wsl"],
    prerequisites: [{ kind: "wsl2", reason: "Nikto is a Linux tool and runs inside a WSL 2 distribution.", requires_admin: true, requires_restart: true }],
    install: [{ method: "wsl_apt", packages: ["nikto"] }],
  }),
  tool({
    id: "trivy",
    name: "Trivy",
    summary: "Vulnerability and misconfiguration scanner.",
    category: "vulnerability",
    install: [{ method: "winget", package_id: "AquaSecurity.Trivy" }],
    launch_profiles: [
      {
        ...nativeProfile("scan-folder", "Scan a project folder for known vulnerabilities and exposed secrets", "Reads the dependency lists and files in a folder on this computer.", "trivy.exe", [
          { name: "folder", label: "Folder to scan", description: "A project folder you own.", kind: { type: "path", path_kind: "directory", must_exist: true }, required: true, default: "C:\Demo\Vixalune\labs\sample-code" },
          { name: "severity", label: "Report severities", description: "Which severities to list.", kind: { type: "choice", options: ["CRITICAL", "HIGH,CRITICAL", "MEDIUM,HIGH,CRITICAL"] }, required: true, default: "HIGH,CRITICAL" },
        ], "Tables of vulnerable packages and any secrets found. Exit code 1 means findings, not a crash.", 900),
        network_activity: "downloads_data",
        exit_codes: { ok: [0], findings: [1] },
      },
    ],
  }),
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
    ["nikto", "not_installed"],
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

  // Demo platform: Windows tools ready; WSL and containers need setup (so the wizard can be explored).
  const platform = { wsl: false, distro: false, restartPending: false, restartScheduled: false };
  const demoActions = (): SetupAction[] => {
    const wsl: SetupAction = {
      id: "install_wsl",
      title: "Install Windows Subsystem for Linux (WSL 2)",
      why: "Linux-based security tools run inside WSL 2, and Docker Desktop's Linux engine uses it on Windows Home.",
      kind: "automatic",
      requires_admin: true,
      requires_restart: true,
      command_preview: "wsl.exe --install --no-distribution",
      official_link: "https://learn.microsoft.com/windows/wsl/install",
      instructions: ["Windows will show an administrator (UAC) prompt for this one command.", "Windows may ask you to restart afterwards. Vixalune never restarts without asking you first."],
      depends_on: [],
    };
    const distro: SetupAction = {
      id: "install_distro",
      title: "Install a Linux distribution (Ubuntu 24.04)",
      why: "WSL needs at least one Linux distribution to hold Linux tools.",
      kind: "automatic",
      requires_admin: false,
      requires_restart: false,
      command_preview: "wsl.exe --install --distribution Ubuntu-24.04 --no-launch",
      official_link: "https://learn.microsoft.com/windows/wsl/install",
      instructions: ["Downloads Ubuntu from Microsoft's distribution list (a few hundred MB)."],
      depends_on: ["install_wsl"],
    };
    const docker: SetupAction = {
      id: "start_docker_desktop",
      title: "Start Docker Desktop",
      why: "Docker is installed but its engine is not running.",
      kind: "manual",
      requires_admin: false,
      requires_restart: false,
      command_preview: null,
      official_link: "https://docs.docker.com/desktop/troubleshoot-and-support/troubleshoot/",
      instructions: ["Open Docker Desktop from the Start menu and wait until it reports the engine is running, then re-check."],
      depends_on: [],
    };
    return [...(platform.wsl ? [] : [wsl]), ...(platform.distro ? [] : [distro]), ...(platform.distro ? [] : [docker])];
  };
  const demoPlatform = (): PlatformReport => {
    const ready = (s: string): CapabilityReadiness => ({ state: "ready", summary: s, missing: [], blockers: [], notes: [] });
    const wslReady = platform.wsl && platform.distro;
    return {
      detected_at: new Date().toISOString(),
      windows: { build: 26100, display_version: "24H2", edition: "Core", architecture: "x64" },
      virtualization: { firmware_enabled: true, hypervisor_present: true },
      features: [
        { id: "virtualization_firmware", label: "CPU virtualization (VT-x / AMD-V)", state: "available", basis: "CPU feature flag reported by Windows" },
        { id: "hypervisor", label: "Windows hypervisor (used by WSL 2 and Docker Desktop)", state: "available", basis: "CPUID hypervisor bit" },
        { id: "wsl", label: "Windows Subsystem for Linux", state: platform.wsl ? "available" : "unavailable", basis: "wsl.exe responds to --status/--version" },
        { id: "optional_features", label: "Virtual Machine Platform / WSL optional components", state: "unknown", basis: "Reading Windows optional features needs administrator rights; Vixalune never elevates just to detect, so these are inferred from the rows above" },
      ],
      wsl: {
        installed: platform.wsl,
        detail: platform.wsl ? null : "The Windows Subsystem for Linux is not installed.",
        wsl_version: platform.wsl ? "2.3.26.0" : null,
        default_version: platform.wsl ? 2 : null,
        distros: platform.distro ? [{ name: "Ubuntu-24.04", state: "running", version: 2, is_default: true, internal: false, os_id: "ubuntu" }] : [],
      },
      containers: {
        runtime: {
          name: "docker",
          cli_path: "C:\\Demo\\Docker\\docker.exe",
          cli_version: "29.8.2",
          context: "desktop-linux",
          context_local: true,
          daemon: { state: "stopped", reason: "failed to connect to the docker API; check if the daemon is running" },
        },
      },
      package_managers: [{ name: "winget", scope: "Windows", version: "v1.9 (demo)" }],
      disk: { path: "C:\\Demo\\Vixalune", free_bytes: 600 * 1024 ** 3, total_bytes: 1000 * 1024 ** 3 },
      system: { pending_restart: platform.restartPending, proxy: null, policy_keys: [], is_elevated: false },
      configured_distro: null,
      readiness: {
        windows_tools: ready("Ready to install Windows tools"),
        linux_tools_wsl: wslReady
          ? { ...ready("A WSL 2 distribution is ready"), notes: ["WSL shares your Windows files and network. It is not a security sandbox: only run tools you trust."] }
          : {
              state: "needs_setup",
              summary: platform.wsl ? "No Linux distribution is installed" : "WSL is not installed",
              missing: platform.wsl ? ["A Linux distribution"] : ["Windows Subsystem for Linux", "A Linux distribution"],
              blockers: [],
              notes: [],
            },
        containers: {
          state: "needs_setup",
          summary: "The container engine is not running",
          missing: ["A running container engine"],
          blockers: [],
          notes: platform.wsl ? [] : ["Docker Desktop needs WSL 2 (or Hyper-V on Pro/Enterprise editions) to start its Linux engine; WSL is not installed here."],
        },
      },
      setup_actions: demoActions(),
    };
  };

  const storage: StorageInfo = { data_dir: "C:\\Demo\\AppData", config_path: "C:\\Demo\\AppData\\config.toml", database_path: "C:\\Demo\\AppData\\vixalune.db", runs_dir: "C:\\Demo\\AppData\\runs", tools_dir: "C:\\Demo\\AppData\\tools", labs_dir: "C:\\Demo\\Vixalune\\labs", data_dir_overridden: false, restart_required: false };
  const runs = createDemoRuns({ tools: DEMO_TOOLS, jobs, output, nextId: () => `demo-job-${++counter}` });

  return {
    isDemo: true,
    getAppInfo: async () => ({ name: "Vixalune", version: "demo", data_dir: "(demo)", rejected_manifests: [] }),
    getCapabilities: async () => ({
      install: true,
      uninstall: true,
      verify: true,
      launch: true,
      jobs: true,
      labs: true,
      installed_detection: true,
      install_methods: ["winget", "wsl_apt", "container_image"],
    }),
    getPlatformReport: async () => demoPlatform(),
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
    getJobOutput: async (jobId) => (runs.rows[jobId] ? runs.rows[jobId]!.map((r) => r.text) : (output[jobId] ?? [])),
    recentActivity: async () => [
      { at: minutesAgo(1), kind: "app_started", summary: "Demo: Vixalune started" },
      { at: minutesAgo(10), kind: "install_failed", tool_id: "zap", summary: "Demo: verification of ZAP failed" },
      { at: minutesAgo(31), kind: "install_succeeded", tool_id: "nmap", summary: "Demo: installed Nmap" },
    ],
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
        notes: m.platforms.includes("wsl") ? ["WSL is not a sandbox: Linux programs can read and change your Windows files and use your network."] : m.platforms.includes("container") ? ["A container is not a safe place to run hostile code."] : [],
        unsupported_reason: docker ? "Demo: this tool needs the container installer, which is not available." : null,
      };
    },
    installTool: async (id, allowElevation) => need(id) ?? simulate(id, "install", allowElevation, "installed"),
    uninstallTool: async (id, allowElevation) => need(id) ?? simulate(id, "uninstall", allowElevation, "not_installed"),
    verifyTool: async (id) => need(id) ?? simulate(id, "verify", false, "installed"),
    ...runs,
    runSetupAction: async (id, confirmed) => {
      if (!confirmed) return Promise.reject({ code: "validation", message: "validation failed: this changes your computer, so it needs your explicit confirmation", recovery: "Correct the highlighted value and try again." });
      const jobId = `demo-job-${++counter}`;
      const toolId = id === "install_wsl" ? "setup-install-wsl" : "setup-install-distro";
      const j = job({ id: jobId, tool_id: toolId, kind: "setup", phase: "installing", message: "Demo: running the setup step…", allow_elevation: true });
      jobs.unshift(j);
      output[jobId] = ["[demo] This is simulated output; nothing was changed."];
      setTimeout(() => {
        if (id === "install_wsl") {
          platform.wsl = true;
          platform.restartPending = true;
        } else {
          platform.distro = true;
        }
        Object.assign(j, { phase: "succeeded" satisfies JobPhase, message: id === "install_wsl" ? "Demo: WSL was installed. Restart Windows to finish; Vixalune will not restart it without your confirmation." : "Demo: distribution installed", finished_at: new Date().toISOString(), exit_code: 0 });
      }, 1500);
      return jobId;
    },
    requestRestart: async (confirmed) => {
      if (!confirmed) return Promise.reject({ code: "validation", message: "validation failed: restarting Windows needs your explicit confirmation", recovery: "Correct the highlighted value and try again." });
      platform.restartScheduled = true; // Demo only: nothing is restarted.
    },
    cancelRestart: async () => {
      platform.restartScheduled = false;
      platform.restartPending = false;
    },
    openSetupLink: async () => undefined,
    approveJob: async (jobId) => {
      const j = jobs.find((x) => x.id === jobId);
      if (j?.phase === "awaiting_consent") j.phase = "installing";
    },
    cancelJob: async (jobId) => {
      if (runs.cancelRun(jobId)) return;
      const j = jobs.find((x) => x.id === jobId);
      if (j) {
        j.phase = "cancelled";
        j.finished_at = new Date().toISOString();
        states.set(j.tool_id, "not_installed");
      }
    },
    getStorageInfo: async () => structuredClone(storage),
    updateStorage: async (u) => {
      if (![u.tools_dir, u.labs_dir].every((p) => /^[A-Za-z]:\\[^<>"|?*]{2,}$/.test(p))) throw { code: "validation", message: "validation failed: use a full folder path such as D:\\Vixalune\\labs", recovery: "Correct the highlighted value and try again." };
      Object.assign(storage, { tools_dir: u.tools_dir, labs_dir: u.labs_dir, restart_required: true });
      return structuredClone(storage);
    },
    openReleasePage: async () => undefined,
    openLink: async () => {
      // Demo URLs are placeholders; nothing is opened.
    },
    // The demonstration never pretends to talk to an AI provider or to store a key.
    aiOverview: noAi,
    aiSaveCredential: noAi,
    aiDeleteCredential: noAi,
    aiUpdateConfig: noAi,
    aiSetActive: noAi,
    aiUpdateSettings: noAi,
    aiListModels: noAi,
    aiTestConnection: noAi,
    aiCreateConversation: noAi,
    aiListConversations: noAi,
    aiGetConversation: noAi,
    aiDeleteConversation: noAi,
    aiDeleteAllConversations: noAi,
    aiSendMessage: noAi,
    aiRetry: noAi,
    aiCancel: noAi,
    aiOpenProviderPage: noAi,
    onAiEvent: async () => () => {},
  };
}




function noAi(): Promise<never> {
  return Promise.reject({
    code: "demo_ai",
    message: "AI providers are not available in the demonstration.",
    recovery: "Run the desktop app (npm run tauri:dev or the installed app) to add a provider and chat. Vixalune never shows invented AI answers.",
  });
}
