import type { Backend } from "@/domain/backend";
import type {
  CatalogEntry,
  Job,
  LabState,
  LabStatus,
  Locality,
  OutputRow,
  ParamValue,
  RunApprovals,
  RunOutcome,
  RunPreview,
  RunRecord,
  RunSpec,
  ScopeReport,
  TargetAssessment,
  Tutorial,
  UserSettings,
} from "@/domain/types";

/**
 * DEMONSTRATION DATA ONLY (see demoBackend). Simulates tool runs, the scope check, labs, settings and tutorials so the
 * interface can be explored without running anything. None of this reflects real behaviour of the real backend.
 */

const validation = (message: string) => ({ code: "validation", message: `validation failed: ${message}`, recovery: "Correct the highlighted value and try again." });
const notFound = (what: string) => ({ code: "not_found", message: `${what} was not found`, recovery: "Refresh the page and try again." });

type Deps = {
  tools: CatalogEntry[];
  jobs: Job[];
  output: Record<string, string[]>;
  nextId: () => string;
};

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;

function classifyHost(host: string, lab: string[]): { locality: Locality; addresses: string[] } {
  const h = host.toLowerCase();
  const m = IPV4.exec(h);
  let locality: Locality;
  let addresses = [h];
  if (m) {
    const [a, b] = [Number(m[1]), Number(m[2])];
    locality = a === 127 ? "loopback" : a === 10 || (a === 192 && b === 168) || (a === 172 && b >= 16 && b <= 31) ? "private_network" : a === 0 || a >= 224 ? "special" : "public";
  } else if (h === "localhost") {
    locality = "loopback";
    addresses = ["127.0.0.1"];
  } else if (h.endsWith(".lab.test")) {
    locality = "private_network";
    addresses = ["192.168.56.10"];
  } else {
    locality = "public";
    addresses = ["93.184.216.34"];
  }
  if (locality === "private_network" && (lab.includes(h) || lab.some((l) => l.startsWith("192.168.56.") && h.startsWith("192.168.56.")))) locality = "lab";
  return { locality, addresses };
}

const VERDICT = { loopback: "allowed", lab: "allowed", private_network: "needs_confirmation", public: "needs_confirmation", unspecified: "blocked", special: "blocked", unresolvable: "blocked" } as const;

function assess(parameter: string, form: "host" | "ip" | "url", value: string, lab: string[]): TargetAssessment {
  const raw = value.trim();
  if (!raw || /[\s;&|`$<>,\\]/.test(raw) || raw.startsWith("-")) throw validation(`parameter '${parameter}': '${raw}' is not a valid ${form === "url" ? "web address" : "host name or IP address"}`);
  let host = raw;
  if (form === "url") {
    let u: URL;
    try {
      u = new URL(raw);
    } catch {
      throw validation(`parameter '${parameter}': '${raw}' is not a valid web address`);
    }
    if (u.protocol !== "http:" && u.protocol !== "https:") throw validation(`parameter '${parameter}': only http and https addresses are accepted`);
    if (u.username || u.password) throw validation(`parameter '${parameter}': addresses with a user name or password are not accepted`);
    host = u.hostname;
  }
  const { locality, addresses } = classifyHost(host, lab);
  const reasons = {
    loopback: "This computer (loopback). Always in scope.",
    lab: "On your configured lab list.",
    private_network: "A private-network address that is not on your lab list. Only continue if you own or are authorised to test it.",
    public: "A public internet address. Only continue if you own it or have written permission to test it.",
    unspecified: "Not a specific target.",
    special: "A multicast, broadcast or reserved address; it cannot be a test target.",
    unresolvable: "Unresolvable.",
  }[locality];
  return { parameter, raw, canonical: form === "url" ? raw : host.toLowerCase(), form, addresses, locality, verdict: VERDICT[locality], reasons: [reasons] };
}

function scopeFor(profile: RunSpec, values: Record<string, ParamValue>, lab: string[]): ScopeReport | null {
  const targets: TargetAssessment[] = [];
  for (const p of profile.parameters) {
    const k = p.kind.type;
    if (k !== "host" && k !== "ip" && k !== "url") continue;
    const v = values[p.name];
    if (typeof v !== "string" || v === "") continue;
    targets.push(assess(p.name, k, v, lab));
  }
  if (targets.length === 0) return null;
  const rank = { allowed: 0, needs_confirmation: 1, blocked: 2 } as const;
  const verdict = targets.reduce<keyof typeof rank>((m, t) => (rank[t.verdict] > rank[m] ? t.verdict : m), "allowed");
  const publicOne = targets.find((t) => t.verdict === "needs_confirmation" && t.locality === "public");
  const hostOf = (t: TargetAssessment) => (t.form === "url" ? new URL(t.canonical).hostname : t.canonical);
  return {
    targets,
    verdict,
    confirmation: verdict === "needs_confirmation" ? (publicOne ? { level: "type_target", phrase: hostOf(publicOne) } : { level: "acknowledge", phrase: null }) : null,
    blocked_reasons: targets.filter((t) => t.verdict === "blocked").map((t) => `${t.canonical}: ${t.reasons.join(" ")}`),
    redirects: [],
    lab_targets: lab,
  };
}

const sortedAddresses = (r: ScopeReport) => [...new Set(r.targets.flatMap((t) => t.addresses))].sort();

function enforce(report: ScopeReport | null, approvals: RunApprovals) {
  if (!report) return;
  if (report.verdict === "blocked") throw validation(`out of scope: ${report.blocked_reasons.join(" | ")}`);
  if (report.verdict !== "needs_confirmation") return;
  const ack = approvals.scope;
  if (!ack?.acknowledged) throw validation("this target is outside your lab: confirm that you are authorised to test it before running");
  if (report.confirmation?.level === "type_target" && (ack.typed ?? "").trim().toLowerCase() !== (report.confirmation.phrase ?? "").toLowerCase())
    throw validation(`type the target exactly (${report.confirmation.phrase}) to confirm you are authorised to test it`);
  if (JSON.stringify([...ack.addresses].sort()) !== JSON.stringify(sortedAddresses(report))) throw validation("the addresses this target resolves to changed since you reviewed them; review and confirm again");
}

const DEMO_OUTPUT: Record<string, { lines: string[]; exit: number; outcome: RunOutcome }> = {
  "nmap/quick-scan": { lines: ["Starting Nmap (demo)", "Nmap scan report for localhost (127.0.0.1)", "PORT    STATE SERVICE", "135/tcp open  msrpc", "445/tcp open  microsoft-ds", "Nmap done: 1 IP address (1 host up) scanned in 1.2 seconds"], exit: 0, outcome: "findings" },
  "nmap/service-scan": { lines: ["Starting Nmap (demo)", "Nmap scan report for localhost (127.0.0.1)", "PORT      STATE SERVICE VERSION", "18080/tcp open  http    (demo)", "Nmap done: 1 IP address (1 host up) scanned in 6.4 seconds"], exit: 0, outcome: "findings" },
  "zap/quick-scan": { lines: ["Spider progress: 100%", "Active scan progress: 100%", "Report written: zap-report.html", "Medium  Content Security Policy (CSP) header not set (demo)"], exit: 0, outcome: "findings" },
  "trivy/scan-folder": { lines: ["requirements.txt (pip)", "Total: 5 (HIGH: 3, CRITICAL: 2) (demo)"], exit: 1, outcome: "findings" },
  "semgrep/scan-sample": { lines: ["Ran 3 rules on 6 files: 3 findings. (demo)", "app.py: practice-hardcoded-password", "app.py: practice-eval-of-input"], exit: 1, outcome: "findings" },
};

function demoTutorial(toolId: string, name: string, profile: string, lab: string): Tutorial {
  return {
    tool_id: toolId,
    title: `Learn ${name} (demo content)`,
    what_it_does: `${name} is shown here with demonstration text. The real application ships tutorials written for each tool.`,
    problem_solved: "Demo: explains which problem the tool solves.",
    requirements: [`${name} installed from the Tool Library.`, "A target you own or have written permission to test."],
    authorization_warning: "Only test systems you own or have written permission to test. Demo text.",
    first_run: [
      { title: "Start the practice lab", body: "Open Learning Labs and start the lab." },
      { title: "Open the launcher", body: "Go to Run a Tool and pick the profile.", profile },
      { title: "Read the preview, then run", body: "Nothing runs until you press Run.", profile },
    ],
    parameters: [{ profile, name: "target", explanation: "Demo: what this field means." }],
    example: { profile, output: "Demo output (illustrative, not a recording)", interpretation: ["Demo: how to read the result."] },
    common_errors: [
      { symptom: "Demo error one", cause: "Demo cause", fix: "Demo fix" },
      { symptom: "Demo error two", cause: "Demo cause", fix: "Demo fix" },
    ],
    exercise: { title: "Demo exercise", lab, profile, goal: "Demo goal.", steps: ["Demo step."], expected: "Demo expectation." },
    links: [{ label: `${name} documentation (demo link)`, url: `https://example.invalid/${toolId}/docs` }],
  };
}

export function createDemoRuns(d: Deps): Pick<
  Backend,
  | "listLabs"
  | "startLab"
  | "stopLab"
  | "getLaunchDefaults"
  | "getJob"
  | "getRunOutput"
  | "listRuns"
  | "getRun"
  | "saveRunOutput"
  | "openRunFolder"
  | "getSettings"
  | "updateSettings"
  | "purgeHistoryNow"
  | "listTutorials"
  | "getTutorial"
  | "openTutorialLink"
  | "previewLaunch"
  | "launchTool"
> & { runs: RunRecord[]; rows: Record<string, OutputRow[]>; cancelRun: (id: string) => boolean } {
  const runs: RunRecord[] = [];
  const rows: Record<string, OutputRow[]> = {};
  const timers: Record<string, ReturnType<typeof setTimeout>[]> = {};
  let settings: UserSettings = { scope: { lab_targets: [] }, retention: { history_days: 90, output_days: 14 } };
  const labStates: Record<string, LabState> = { "practice-web": "stopped", "sample-code": "running" };

  const labs = (): LabStatus[] => [
    {
      id: "practice-web",
      name: "Practice web application",
      description: "A small shop website with deliberate, harmless weaknesses. It runs only on this computer (127.0.0.1) and serves fixed pages. (Demo)",
      kind: "web_app",
      address: labStates["practice-web"] === "running" ? "http://127.0.0.1:18080/" : "http://127.0.0.1:18080/ (when started)",
      folder: null,
      weaknesses: ["No security headers (CSP, X-Frame-Options, X-Content-Type-Options, HSTS).", "The session cookie has no HttpOnly, Secure or SameSite attributes.", "/search reflects the 'q' parameter without escaping.", "/files/ shows a directory listing with a backup file.", "/admin/ has no login."],
      warning: "Practise only on this lab or on systems you own or have written permission to test.",
      state: labStates["practice-web"]!,
    },
    {
      id: "sample-code",
      name: "Sample code project",
      description: "Insecure sample code and out-of-date dependency lists for Semgrep and Trivy practice. The files are never executed. (Demo)",
      kind: "sample_project",
      address: null,
      folder: "C:\\Demo\\Vixalune\\labs\\sample-code",
      weaknesses: ["Hard-coded password, eval() on user input and a shell command built from input (app.py).", "Old Flask, Requests, Jinja2 and lodash versions with known vulnerabilities.", "A fake access key in config.env."],
      warning: "Never run these files. They are intentionally unsafe examples.",
      state: labStates["sample-code"]!,
    },
  ];

  const find = (id: string, profile: string): { entry: CatalogEntry; p: RunSpec } => {
    const entry = d.tools.find((t) => t.manifest.id === id);
    const p = entry?.manifest.launch_profiles.find((x) => x.id === profile);
    if (!entry || !p) throw notFound(`launch profile '${profile}' of '${id}'`);
    return { entry, p };
  };

  const effective = (p: RunSpec, values: Record<string, ParamValue>) => {
    const v: Record<string, ParamValue> = {};
    for (const def of p.parameters) {
      const given = values[def.name] ?? def.default ?? undefined;
      if (given !== undefined && given !== "") v[def.name] = given;
    }
    for (const k of Object.keys(values)) if (!p.parameters.some((def) => def.name === k)) throw validation(`unknown parameter '${k}'`);
    return v;
  };

  const display = (p: RunSpec, v: Record<string, ParamValue>) => `${p.executable} ${Object.values(v).join(" ")}`.trim();

  const finish = (id: string, outcome: RunOutcome, exit: number | null) => {
    const j = d.jobs.find((x) => x.id === id);
    const r = runs.find((x) => x.id === id);
    if (!j || !r) return;
    const now = new Date().toISOString();
    Object.assign(j, {
      phase: outcome === "cancelled" ? "cancelled" : outcome === "failed" || outcome === "timed_out" ? "failed" : "succeeded",
      message: outcome === "findings" ? "Finished: the tool reported findings" : outcome === "clean" ? "Finished: nothing reported" : outcome === "cancelled" ? "Cancelled; the process was stopped" : "Failed",
      finished_at: now,
      updated_at: now,
      exit_code: exit,
    });
    Object.assign(r, { finished_at: now, duration_ms: Date.parse(now) - Date.parse(r.started_at), exit_code: exit, outcome });
  };

  return {
    runs,
    rows,
    cancelRun: (id) => {
      const r = runs.find((x) => x.id === id);
      if (!r || r.outcome) return false;
      (timers[id] ?? []).forEach(clearTimeout);
      finish(id, "cancelled", null);
      return true;
    },
    listLabs: async () => labs(),
    startLab: async (id) => {
      if (!(id in labStates)) throw notFound(`lab '${id}'`);
      labStates[id] = "running";
      return labs();
    },
    stopLab: async (id) => {
      if (!(id in labStates)) throw notFound(`lab '${id}'`);
      if (id === "practice-web") labStates[id] = "stopped";
      return labs();
    },
    getLaunchDefaults: async (toolId, profile) => {
      const { p } = find(toolId, profile);
      return {
        parameters: Object.fromEntries(p.parameters.filter((x) => x.default != null).map((x) => [x.name, x.default as ParamValue])),
        mounts: Object.fromEntries((p.suggested_mounts ?? []).map((m) => [m.name, m.value])),
      };
    },
    getJob: async (id) => {
      const j = d.jobs.find((x) => x.id === id);
      if (!j) throw notFound(`job '${id}'`);
      return { ...j };
    },
    getRunOutput: async (id, after) => (rows[id] ?? []).filter((r) => r.seq > after),
    listRuns: async (limit) => runs.slice(0, limit ?? 50).map((r) => ({ ...r })),
    getRun: async (id) => {
      const r = runs.find((x) => x.id === id);
      if (!r) throw notFound(`run '${id}'`);
      return { ...r };
    },
    saveRunOutput: async (id) => {
      const r = runs.find((x) => x.id === id);
      if (!r) throw notFound(`run '${id}'`);
      r.result_path = `C:\\Demo\\Documents\\${r.tool_id}-${r.profile_id}.txt`;
      return r.result_path;
    },
    openRunFolder: async () => undefined,
    getSettings: async () => structuredClone(settings),
    updateSettings: async (s) => {
      if (s.retention.history_days < 1 || s.retention.history_days > 3650) throw validation("history retention must be 1-3650 days");
      if (s.retention.output_days > s.retention.history_days) throw validation("output cannot be kept longer than the history that refers to it");
      if (s.scope.lab_targets.some((t) => /^(0\.0\.0\.0\/0|10\.0\.0\.0\/8)$/.test(t.trim()))) throw validation("that range is too wide to be a lab");
      settings = structuredClone(s);
      return structuredClone(settings);
    },
    purgeHistoryNow: async () => {
      const n = runs.length;
      runs.length = 0;
      for (const k of Object.keys(rows)) delete rows[k];
      return { runs_deleted: n, output_lines_deleted: n * 4, jobs_deleted: 0 };
    },
    listTutorials: async () => [demoTutorial("nmap", "Nmap", "service-scan", "practice-web"), demoTutorial("zap", "OWASP ZAP", "quick-scan", "practice-web"), demoTutorial("semgrep", "Semgrep", "scan-sample", "sample-code"), demoTutorial("trivy", "Trivy", "scan-folder", "sample-code")],
    getTutorial: async (toolId) => {
      const all: Record<string, Tutorial> = {
        nmap: demoTutorial("nmap", "Nmap", "service-scan", "practice-web"),
        zap: demoTutorial("zap", "OWASP ZAP", "quick-scan", "practice-web"),
        semgrep: demoTutorial("semgrep", "Semgrep", "scan-sample", "sample-code"),
        trivy: demoTutorial("trivy", "Trivy", "scan-folder", "sample-code"),
      };
      const t = all[toolId];
      if (!t) throw notFound(`a tutorial for '${toolId}'`);
      return t;
    },
    openTutorialLink: async () => undefined,
    previewLaunch: async (id, profile, values, approvals): Promise<RunPreview> => {
      const { p } = find(id, profile);
      const v = effective(p, values);
      if (p.runtime === "container") {
        if (p.needs_network && !approvals.network) throw validation("this profile needs network access, which you have not approved");
        const missing = p.mounts.find((m) => !approvals.mounts[m]);
        if (missing) throw validation(`choose and approve a folder for '${missing}'`);
        return {
          command: { runtime: "container", program: p.executable, args: ["scan"], display: `${p.executable} scan /src` },
          container: {
            image: "docker.io/semgrep/semgrep@sha256:0000…",
            command: `${p.executable} scan /src`,
            user: "65532:65532",
            network: p.needs_network ? "bridge" : "none",
            mounts: p.mounts.map((m) => ({ host: approvals.mounts[m] ?? "", container: "/src", read_only: true })),
            ports: [],
            limits: "1024 MB memory, 1.00 CPU, 256 processes",
            read_only_rootfs: true,
            warnings: p.needs_network ? ["The container can reach the internet and your local network."] : [],
          },
          scope: null,
          network_activity: p.network_activity ?? "none",
          timeout_secs: p.timeout_secs ?? 900,
          output_format: p.output_format ?? "text",
          expected_output: p.expected_output ?? "",
          working_dir: "container",
          notes: [],
        };
      }
      const scope = scopeFor(p, v, settings.scope.lab_targets);
      if (scope?.verdict === "blocked") throw validation(`out of scope: ${scope.blocked_reasons.join(" | ")}`);
      return {
        command: { runtime: p.runtime, program: p.executable, args: Object.values(v).map(String), display: display(p, v) },
        container: null,
        scope,
        network_activity: p.network_activity ?? "none",
        timeout_secs: p.timeout_secs ?? 600,
        output_format: p.output_format ?? "text",
        expected_output: p.expected_output ?? "",
        working_dir: "A fresh empty folder for this run (the only place the tool may write result files).",
        notes: ["Runs with your normal Windows permissions. Vixalune never asks for administrator rights for a run.", `The run stops by itself after ${p.timeout_secs ?? 600} seconds.`],
      };
    },
    launchTool: async (id, profile, values, approvals) => {
      const { entry, p } = find(id, profile);
      const v = effective(p, values);
      let scope: ScopeReport | null = null;
      if (p.runtime === "container") {
        if (p.needs_network && !approvals.network) throw validation("this profile needs network access, which you have not approved");
        if (p.mounts.some((m) => !approvals.mounts[m])) throw validation("choose and approve a folder for the shared mount");
      } else {
        scope = scopeFor(p, v, settings.scope.lab_targets);
        enforce(scope, approvals);
      }
      if (d.jobs.some((j) => j.tool_id === id && !["succeeded", "failed", "cancelled"].includes(j.phase))) throw { code: "busy", message: `another operation for '${id}' is still running`, recovery: "Wait for the running operation to finish, or cancel it from Running Jobs, then try again." };
      const jobId = d.nextId();
      const started = new Date().toISOString();
      d.jobs.unshift({ id: jobId, tool_id: id, kind: "run", phase: "running", message: `Running ${display(p, v)}`, started_at: started, updated_at: started, allow_elevation: false });
      const scripted = DEMO_OUTPUT[`${id}/${profile}`] ?? { lines: ["Demo: finished."], exit: 0, outcome: "clean" as RunOutcome };
      rows[jobId] = [{ seq: 1, stream: "stdout", text: `> ${display(p, v)}` }];
      runs.unshift({
        id: jobId,
        tool_id: id,
        tool_name: entry.manifest.name,
        profile_id: p.id,
        profile_label: p.label,
        tool_version: "demo",
        started_at: started,
        command: display(p, v),
        targets: scope?.targets.map((t) => t.canonical) ?? [],
        scope: !scope ? "no network targets" : scope.verdict === "allowed" ? (scope.targets.every((t) => t.locality === "loopback") ? "this computer (loopback)" : "your configured lab") : "confirmed outside the lab",
        network_activity: p.network_activity ?? "none",
        output_format: p.output_format ?? "text",
        run_dir: `C:\\Demo\\Vixalune\\runs\\${jobId}`,
      });
      timers[jobId] = scripted.lines.map((line, i) =>
        setTimeout(() => {
          rows[jobId]!.push({ seq: rows[jobId]!.length + 1, stream: "stdout", text: line });
        }, 250 * (i + 1)),
      );
      timers[jobId]!.push(setTimeout(() => finish(jobId, scripted.outcome, scripted.exit), 250 * (scripted.lines.length + 1)));
      return jobId;
    },
  };
}
