import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { AiConversation, AiMessage, AiOverview, AssistantRecommendation, AssistantTurn } from "@/domain/types";
import { createDemoBackend } from "@/infra/demoBackend";
import { renderApp } from "@/test/render";

const conv: AiConversation = { id: "c1", title: "Trivy question", provider: "fake", model: "m", created_at: "2026-10-20T10:00:00Z", updated_at: "2026-10-20T10:00:00Z" };
const user: AiMessage = { id: "u1", conversation_id: "c1", seq: 1, role: "user", content: "Help me scan my own project", status: "complete", error_code: null, error_message: null, provider: null, model: null, usage: null, created_at: "2026-10-20T10:00:00Z" };
const assistantMsg: AiMessage = { ...user, id: "a1", seq: 2, role: "assistant", content: "plain text copy", provider: "fake", model: "m" };

function overview(): AiOverview {
  return {
    providers: [
      {
        descriptor: {
          id: "fake",
          display_name: "FakeAI",
          capabilities: { streaming: true, tool_calling: false, structured_output: false, model_listing: true, usage_reporting: true, cost_reporting: false, cancellation: true, context_limits: false },
          key_hint: "",
          key_page: "https://example.test/keys",
          docs_page: "https://example.test/docs",
          billing_note: "billed",
          verification_note: "test",
          extra_settings: [],
        },
        config: { provider: "fake", label: null, default_model: "m", timeout_secs: 60, temperature: null, max_output_tokens: null, extra: {} },
        has_key: true,
        key_hint: "sk-••••",
        status: { state: "verified", tested_at: null, error_code: null, message: null, steps: [], models_listed: 1, verified_models: ["m"] },
        active: true,
        credential_error: null,
      },
    ],
    credential_backend: "Windows Credential Manager",
    settings: { history_days: 30, allow_vulnerability_lookup: false, local_only: false, max_request_chars: 200000, confirm_large_chars: 60000, session_request_limit: 200 },
    session: { requests_used: 0, limit: 200 },
  };
}

function card(over: Partial<AssistantRecommendation> = {}): AssistantRecommendation {
  return {
    tool_id: "trivy",
    tool_name: "Trivy",
    role: "primary",
    reasons: ["It reads dependency files and reports known vulnerabilities."],
    model_prerequisites: ["An internet connection for the first database download"],
    model_limitations: [],
    evidence: ["E1"],
    summary: "Vulnerability and misconfiguration scanner.",
    status: { state: "not_installed", label: "Not installed", installed: false, version: null, verified: null },
    verified_prerequisites: [{ name: "Winget", reason: "Installs Trivy from the Windows Package Manager.", requires_admin: false, requires_restart: false }],
    verified_limitations: ["Downloads data (for example a vulnerability database) when it runs."],
    profile_id: "scan-folder",
    profile_label: "Scan a project folder",
    documentation_url: "https://trivy.dev/latest/docs/",
    license: "Apache-2.0",
    provenance: { publisher: "Aqua Security Software", source_url: "https://github.com/microsoft/winget-pkgs", verified_on: "2026-10-17" },
    actions: [
      { kind: "view_details", enabled: true, reason: null, profile_id: null },
      { kind: "check_prerequisites", enabled: true, reason: null, profile_id: null },
      { kind: "install", enabled: true, reason: null, profile_id: null },
      { kind: "prepare_run", enabled: false, reason: "Install the tool first.", profile_id: "scan-folder" },
    ],
    ...over,
  };
}

function turn(over: Partial<AssistantTurn> = {}): AssistantTurn {
  return {
    version: 1,
    kind: "recommendation",
    task_category: "dependency_vulnerability",
    user_intent: "scan",
    clarifications: [],
    recommendations: [card()],
    no_suitable_tool: null,
    vulnerability: null,
    finding_notes: [],
    explanation: "Trivy can check a project folder.",
    limitations: [],
    next_steps: [],
    proposed_actions: [],
    evidence: [{ id: "E1", kind: "catalog_record", title: "Trivy — Vixalune catalog record (documentation)", url: "https://trivy.dev/latest/docs/", as_of: "2026-10-17", detail: null }],
    warnings: [],
    model_generated: true,
    consent_request: null,
    ...over,
  };
}

function backendWith(t: AssistantTurn, extra: Partial<Backend> = {}): Backend {
  return {
    ...createDemoBackend(),
    isDemo: false,
    aiOverview: async () => overview(),
    aiListModels: async () => ({ models: [{ id: "m", display_name: null, context_window: null, max_output_tokens: null, supports_tools: null, pricing: null, source: "listed" }], from_provider: true, note: "" }),
    aiListConversations: async () => [conv],
    aiGetConversation: async () => ({ conversation: conv, messages: [user, assistantMsg] }),
    assistantTurns: async () => [{ message_id: "a1", turn: t }],
    onAiEvent: async () => () => undefined,
    ...extra,
  };
}

async function openConversation(be: Backend) {
  renderApp("/assistant", be);
  await userEvent.click(await screen.findByRole("button", { name: conv.title }));
  return await screen.findByRole("article", { name: "Recommendation: Trivy" }).catch(() => null);
}

describe("tool-aware assistant answers", () => {
  it("shows a recommendation with its real status, catalog prerequisites, link and the four actions", async () => {
    const c = await openConversation(backendWith(turn()));
    expect(c).not.toBeNull();
    const card = within(c!);
    expect(card.getByText("Not installed")).toBeInTheDocument();
    expect(card.getByText("License Apache-2.0")).toBeInTheDocument();
    expect(card.getByText(/Installs Trivy from the Windows Package Manager/)).toBeInTheDocument();
    expect(card.getByRole("button", { name: /Official documentation/ })).toBeInTheDocument();
    expect(card.getByRole("button", { name: "View Details — Trivy" })).toBeEnabled();
    expect(card.getByRole("button", { name: "Check Prerequisites — Trivy" })).toBeEnabled();
    expect(card.getByRole("button", { name: "Install… — Trivy" })).toBeEnabled();
    expect(card.getByRole("button", { name: "Prepare Scan — Trivy" })).toBeDisabled();
    expect(card.getByText(/Prepare Scan: Install the tool first/)).toBeInTheDocument();
  });

  it("labels model-written text apart from the application's own facts", async () => {
    await openConversation(backendWith(turn()));
    expect(screen.getByText("Explanation written by the AI model — not a verified finding")).toBeInTheDocument();
    expect(screen.getByText("written by the AI model")).toBeInTheDocument();
    expect(screen.getByText("from the Vixalune catalog")).toBeInTheDocument();
  });

  it("checks prerequisites through the backend and changes nothing", async () => {
    const run = vi.fn(async () => ({
      kind: "prerequisites" as const,
      report: { tool_id: "trivy", tool_name: "Trivy", ready: true, items: [{ name: "Winget", reason: "Installs Trivy.", satisfied: true, requires_admin: false, requires_restart: false }], missing: [], dependencies: [], administrator_rights: "None", unsupported_reason: null, notes: [] },
    }));
    const installTool = vi.fn(async () => "x");
    await openConversation(backendWith(turn(), { assistantRunAction: run, installTool }));
    await userEvent.click(screen.getByRole("button", { name: "Check Prerequisites — Trivy" }));
    expect(run).toHaveBeenCalledWith({ action: "check_prerequisites", tool_id: "trivy" });
    expect(await screen.findByRole("status", { name: "Prerequisites for Trivy" })).toHaveTextContent("can be installed on this computer");
    expect(installTool).not.toHaveBeenCalled();
  });

  it("never offers Install for something already installed, and says why", async () => {
    const installed = card({
      status: { state: "installed", label: "Installed", installed: true, version: "0.76.0", verified: true },
      actions: [
        { kind: "view_details", enabled: true, reason: null, profile_id: null },
        { kind: "check_prerequisites", enabled: true, reason: null, profile_id: null },
        { kind: "install", enabled: false, reason: "Already installed.", profile_id: null },
        { kind: "prepare_run", enabled: true, reason: null, profile_id: "scan-folder" },
      ],
    });
    await openConversation(backendWith(turn({ recommendations: [installed] })));
    expect(screen.getByRole("button", { name: "Install… — Trivy" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Prepare Scan — Trivy" })).toBeEnabled();
    expect(screen.getByText(/Installed · 0\.76\.0/)).toBeInTheDocument();
  });

  it("shows database matches with identifiers, fixed versions, timestamps and sources, and separates them from what was not assessed", async () => {
    const t = turn({
      kind: "vulnerability",
      recommendations: [],
      evidence: [{ id: "E1", kind: "vulnerability_database", title: "OSV GHSA-35jh-r3h4-6jhm", url: "https://osv.dev/vulnerability/GHSA-35jh-r3h4-6jhm", as_of: "2026-10-20T10:00:00Z", detail: null }],
      vulnerability: {
        package: { ecosystem: "npm", name: "lodash", version: "4.17.15" },
        lookup: {
          source: "OSV",
          query: "npm lodash 4.17.15",
          queried_at: "2026-10-20T10:00:00Z",
          truncated: false,
          coverage_notes: ["OSV lists publicly reported vulnerabilities."],
          records: [
            {
              id: "GHSA-35jh-r3h4-6jhm",
              aliases: ["CVE-2021-23337"],
              summary: "Command Injection in lodash",
              severity: [{ kind: "CVSS_V3", score: "CVSS:3.1/AV:N" }],
              affected: [{ ecosystem: "npm", package: "lodash", range_type: "SEMVER", introduced: "0", fixed: "4.17.21", last_affected: null }],
              fixed_versions: ["4.17.21"],
              published: "2021-05-06T16:05:51Z",
              modified: "2024-01-01T00:00:00Z",
              references: [],
              source: "OSV",
              source_url: "https://osv.dev/vulnerability/GHSA-35jh-r3h4-6jhm",
              class: "database_match",
            },
          ],
        },
        classes: ["Vulnerability database match: the database lists this version as affected. It does not show that your use of the package is vulnerable."],
        not_assessed: ["Whether your code actually reaches the vulnerable code (confirmed vulnerable configuration): not assessed."],
        scanner_ids: [],
        scanner_note: null,
      },
    });
    await openConversation(backendWith(t));
    const s = await screen.findByRole("region", { name: "Vulnerability findings" });
    expect(s).toHaveTextContent("lodash 4.17.15 (npm)");
    expect(s).toHaveTextContent("GHSA-35jh-r3h4-6jhm");
    expect(s).toHaveTextContent("CVE-2021-23337");
    expect(s).toHaveTextContent("Fixed in: 4.17.21");
    expect(s).toHaveTextContent("Published: 2021-05-06T16:05:51Z");
    expect(s).toHaveTextContent("database match");
    expect(s).toHaveTextContent("not assessed");
    expect(s).toHaveTextContent("Asked OSV at 2026-10-20T10:00:00Z");
    expect(within(s).getByRole("button", { name: /GHSA-35jh-r3h4-6jhm/ })).toBeInTheDocument();
  });

  it("never says 'safe' for an empty result", async () => {
    const t = turn({ kind: "vulnerability", recommendations: [], vulnerability: { package: { ecosystem: "npm", name: "tiny", version: "1.0.0" }, lookup: { source: "OSV", query: "q", queried_at: "t", records: [], truncated: false, coverage_notes: ["No known vulnerability was found in this database. That is not proof that the package is safe."] }, classes: [], not_assessed: [], scanner_ids: [], scanner_note: null } });
    await openConversation(backendWith(t));
    const s = await screen.findByRole("region", { name: "Vulnerability findings" });
    expect(s).toHaveTextContent("not proof that the package is safe");
  });

  it("asks for missing details and for permission before any lookup", async () => {
    const update = vi.fn(async (s) => s);
    const t = turn({
      kind: "needs_information",
      recommendations: [],
      explanation: "I have what I need, but looking it up means sending the package name and version to OSV.",
      consent_request: { kind: "vulnerability_lookup", text: "Allow Vixalune to send the package name and version (for example “lodash 4.17.15”) to OSV and NVD." },
      clarifications: [{ question: "Which version are you using?", why: "Whether a vulnerability applies depends on the exact version." }],
    });
    await openConversation(backendWith(t, { aiUpdateSettings: update }));
    expect(await screen.findByRole("region", { name: "Questions" })).toHaveTextContent("Which version are you using?");
    const perm = screen.getByRole("region", { name: "Permission needed" });
    expect(perm).toHaveTextContent("send the package name and version");
    expect(update).not.toHaveBeenCalled();
    await userEvent.click(within(perm).getByRole("button", { name: "Allow lookups and ask again" }));
    await waitFor(() => expect(update).toHaveBeenCalledWith({ history_days: 30, allow_vulnerability_lookup: true, local_only: false, max_request_chars: 200000, confirm_large_chars: 60000, session_request_limit: 200 }));
    expect(screen.getByLabelText("Message")).toHaveValue("Help me scan my own project");
  });

  it("shows what the application removed from the model's answer", async () => {
    await openConversation(backendWith(turn({ warnings: ["A recommendation for 'metasploit-pro' was removed: there is no such tool in the catalog."] })));
    expect(await screen.findByRole("region", { name: "Removed or refused" })).toHaveTextContent("metasploit-pro");
  });

  it("lists suggested actions as suggestions that have not run", async () => {
    await openConversation(backendWith(turn({ proposed_actions: [{ action: { action: "prepare_tool_run", tool_id: "trivy", profile_id: "scan-folder" }, risk: "prepares", summary: "Prepare “Scan a project folder” with Trivy (nothing starts until you start it)" }] })));
    expect(await screen.findByRole("region", { name: "Suggested actions" })).toHaveTextContent("none has run");
  });

  it("names tools that are not in the catalog as not integrated", async () => {
    await openConversation(backendWith(turn({ recommendations: [], no_suitable_tool: { explanation: "Nothing here audits Wi-Fi.", documented_alternatives: [{ name: "Aircrack-ng", why: "A documented Wi-Fi auditing suite" }] } })));
    const s = await screen.findByRole("region", { name: "No suitable catalog tool" });
    expect(s).toHaveTextContent("Documented elsewhere (not part of Vixalune");
    expect(s).toHaveTextContent("Aircrack-ng");
  });
});

describe("asking the tool-aware assistant", () => {
  it("uses the backend assistant by default, with the catalog switch on", async () => {
    const ask = vi.fn(async () => ({ request_id: "r1", conversation_id: "c1", user_message: user, assistant_message_id: "a1" }));
    const send = vi.fn();
    const be = backendWith(turn(), { assistantAsk: ask, aiSendMessage: send, aiListConversations: async () => [], aiCreateConversation: async () => conv });
    renderApp("/assistant", be);
    expect(await screen.findByRole("checkbox", { name: /Use the tool catalog/ })).toBeChecked();
    await waitFor(() => expect(screen.getByLabelText("Model")).toBeEnabled());
    await userEvent.selectOptions(screen.getByLabelText("Model"), "m");
    await userEvent.type(screen.getByLabelText("Message"), "Help me scan my own project");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(ask).toHaveBeenCalledWith({ conversation_id: "c1", text: "Help me scan my own project", provider: "fake", model: "m", confirm_large: false }));
    expect(send).not.toHaveBeenCalled();
    expect(await screen.findByText(/Checking the catalog and drafting an answer/)).toBeInTheDocument();
  });

  it("explaining a run needs the person's explicit confirmation of what is sent", async () => {
    const explain = vi.fn(async () => ({ request_id: "r2", conversation_id: "c1", user_message: user, assistant_message_id: "a2" }));
    const run = { id: "2f0e6a2c-3b0a-4d11-8d3e-6f9d3d2c9a11", tool_id: "trivy", tool_name: "Trivy", profile_id: "scan-folder", profile_label: "Scan a project folder", tool_version: null, started_at: "2026-10-20T10:00:00Z", finished_at: "x", duration_ms: 1, exit_code: 1, outcome: "findings", command: "", targets: [], scope: "", network_activity: "none", output_format: "text", run_dir: null, result_path: null };
    const be = backendWith(turn(), { assistantExplainRun: explain, listRuns: async () => [run as never], aiListConversations: async () => [], aiCreateConversation: async () => conv });
    renderApp("/assistant", be);
    await userEvent.click(await screen.findByRole("button", { name: /Explain a scan run/ }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("will be sent to FakeAI over the internet");
    expect(dialog).toHaveTextContent("credentials withheld");
    expect(explain).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "Send and explain" }));
    await waitFor(() => expect(explain).toHaveBeenCalledWith("c1", run.id, true, "fake", "m"));
  });
});
