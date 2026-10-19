import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { InstallState, PlatformReport } from "@/domain/types";
import { SETUP_SEEN_KEY } from "@/pages/SetupWizard";
import { createDemoBackend } from "@/infra/demoBackend";
import { renderApp } from "@/test/render";

async function basePlatform(): Promise<PlatformReport> {
  return createDemoBackend().getPlatformReport();
}

function backendWith(overrides: Partial<Backend> = {}): Backend {
  return { ...createDemoBackend(), listJobs: async () => [], ...overrides };
}

async function openStep(name: RegExp | string) {
  await userEvent.click(await screen.findByRole("button", { name }));
}

describe("setup wizard", () => {
  beforeEach(() => localStorage.clear());

  it("opens on first launch, once, and never changes anything by itself", async () => {
    const runSetupAction = vi.fn(async () => "x");
    const requestRestart = vi.fn(async () => undefined);
    renderApp("/", backendWith({ runSetupAction, requestRestart }), { firstRunRedirect: true });
    expect(await screen.findByRole("heading", { level: 1, name: "Welcome to Vixalune" })).toBeInTheDocument();
    expect(runSetupAction).not.toHaveBeenCalled();
    expect(requestRestart).not.toHaveBeenCalled();
  });

  it("goes straight to the dashboard once setup has been seen", async () => {
    localStorage.setItem(SETUP_SEEN_KEY, "1");
    renderApp("/", backendWith(), { firstRunRedirect: true });
    expect(await screen.findByRole("heading", { level: 1, name: "Dashboard" })).toBeInTheDocument();
  });

  it("step 1 reports facts and keeps detection separate from permission", async () => {
    renderApp("/setup", backendWith());
    const facts = await screen.findByRole("list", { name: "Detected facts" });
    expect(within(facts).getByText(/not installed/)).toBeInTheDocument(); // WSL
    expect(within(facts).getByText(/installed but not running/)).toBeInTheDocument(); // Docker
    expect(within(facts).getByText(/v1\.9/)).toBeInTheDocument(); // winget
    expect(screen.getByText(/it changes nothing/)).toBeInTheDocument();
    expect(screen.getByText(/Reading Windows optional features needs administrator rights/)).toBeInTheDocument();
  });

  it("warns when Vixalune itself runs as administrator, and shows policies and proxies without bypassing them", async () => {
    const p = await basePlatform();
    p.system = { pending_restart: false, proxy: "proxy.corp:8080", policy_keys: ["HKLM\\SOFTWARE\\Policies\\Microsoft\\Windows\\WSL"], is_elevated: true };
    renderApp("/setup", backendWith({ getPlatformReport: async () => p }));
    expect(await screen.findByText(/running as administrator/)).toBeInTheDocument();
    expect(screen.getByText(/proxy\.corp:8080/)).toBeInTheDocument();
    expect(screen.getByText(/does not bypass policies/)).toBeInTheDocument();
  });

  it("explains why WSL 2 and containers may be needed, including their limits", async () => {
    renderApp("/setup", backendWith());
    await openStep(/2\. What you need/);
    expect(await screen.findByText(/it is not a sandbox/)).toBeInTheDocument();
    expect(screen.getByText(/not a safe place for hostile code/)).toBeInTheDocument();
  });

  it("lists exactly what is missing for the chosen capabilities", async () => {
    renderApp("/setup", backendWith());
    await openStep(/3\. What is missing/);
    expect(await screen.findByText("Missing: Windows Subsystem for Linux")).toBeInTheDocument();
    expect(screen.getByText("Missing: A running container engine")).toBeInTheDocument();
    // Unticking containers in step 2 removes it from the list.
    await openStep(/2\. What you need/);
    await userEvent.click(screen.getByRole("checkbox", { name: /Container tools/ }));
    await openStep(/3\. What is missing/);
    expect(screen.queryByText("Missing: A running container engine")).not.toBeInTheDocument();
  });

  it("separates steps Vixalune can run from steps the user does, with official links", async () => {
    const openSetupLink = vi.fn(async () => undefined);
    renderApp("/setup", backendWith({ openSetupLink }));
    await openStep(/4\. Set up/);
    const wsl = (await screen.findByText("Install Windows Subsystem for Linux (WSL 2)")).closest("div")!.parentElement!;
    expect(within(wsl).getByText("Vixalune can do this")).toBeInTheDocument();
    expect(within(wsl).getByText("Administrator approval")).toBeInTheDocument();
    expect(within(wsl).getByText("wsl.exe --install --no-distribution")).toBeInTheDocument();
    const docker = screen.getByText("Start Docker Desktop").closest("div")!.parentElement!;
    expect(within(docker).getByText("You do this")).toBeInTheDocument();
    expect(within(docker).queryByRole("button", { name: /Review and run/ })).not.toBeInTheDocument();
    // The distribution step waits for WSL.
    const distro = screen.getByText(/Install a Linux distribution/).closest("div")!.parentElement!;
    expect(within(distro).getByRole("button", { name: /Review and run/ })).toBeDisabled();
    expect(within(distro).getByText(/Finish the step before this one first/)).toBeInTheDocument();
    await userEvent.click(within(wsl).getByRole("button", { name: /Official guide/ }));
    expect(openSetupLink).toHaveBeenCalledWith("install_wsl");
  });

  it("asks for explicit confirmation showing the exact command before changing anything", async () => {
    const runSetupAction = vi.fn(async () => "job-1");
    renderApp("/setup", backendWith({ runSetupAction }));
    await openStep(/4\. Set up/);
    const wsl = (await screen.findByText("Install Windows Subsystem for Linux (WSL 2)")).closest("div")!.parentElement!;
    await userEvent.click(within(wsl).getByRole("button", { name: /Review and run/ }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("wsl.exe --install --no-distribution")).toBeInTheDocument();
    expect(within(dialog).getByText(/administrator approval \(UAC\)/)).toBeInTheDocument();
    expect(within(dialog).getByText(/never restarts by itself/)).toBeInTheDocument();
    const run = within(dialog).getByRole("button", { name: "Run this step" });
    expect(run).toBeDisabled();
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(runSetupAction).not.toHaveBeenCalled();
    // Confirm for real.
    await userEvent.click(within(wsl).getByRole("button", { name: /Review and run/ }));
    const again = await screen.findByRole("dialog");
    await userEvent.click(within(again).getByRole("checkbox"));
    await userEvent.click(within(again).getByRole("button", { name: "Run this step" }));
    await waitFor(() => expect(runSetupAction).toHaveBeenCalledWith("install_wsl", true));
    expect(runSetupAction).toHaveBeenCalledTimes(1);
  });

  it("shows actionable errors from a failed step", async () => {
    const runSetupAction = vi.fn(() =>
      Promise.reject({ code: "missing_permissions", message: "Permission was denied: you declined the administrator prompt; nothing was changed", recovery: "If it needs administrator rights, approve the Windows prompt when it appears." }),
    );
    renderApp("/setup", backendWith({ runSetupAction }));
    await openStep(/4\. Set up/);
    const wsl = (await screen.findByText("Install Windows Subsystem for Linux (WSL 2)")).closest("div")!.parentElement!;
    await userEvent.click(within(wsl).getByRole("button", { name: /Review and run/ }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("checkbox"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Run this step" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("declined the administrator prompt");
  });

  it("never restarts without a separate explicit confirmation, and the restart can be cancelled", async () => {
    const p = await basePlatform();
    p.system.pending_restart = true;
    const requestRestart = vi.fn(async () => undefined);
    const cancelRestart = vi.fn(async () => undefined);
    renderApp("/setup", backendWith({ getPlatformReport: async () => p, requestRestart, cancelRestart }));
    await openStep(/5\. Restart/);
    expect(await screen.findByText(/Nothing restarts unless you choose it here/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Restart Windows…" }));
    const dialog = await screen.findByRole("dialog", { name: "Restart Windows?" });
    expect(within(dialog).getByText(/Unsaved work/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(requestRestart).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Restart Windows…" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Schedule restart" }));
    await waitFor(() => expect(requestRestart).toHaveBeenCalledWith(true, 60));
    expect(await screen.findByText(/restart is scheduled/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel the restart" }));
    await waitFor(() => expect(cancelRestart).toHaveBeenCalledTimes(1));
  });

  it("says no restart is needed when none is pending", async () => {
    renderApp("/setup", backendWith());
    await openStep(/5\. Restart/);
    expect(await screen.findByText("No restart is needed right now.")).toBeInTheDocument();
  });

  it("re-checks and reports what is still missing, then finishes", async () => {
    const getPlatformReport = vi.fn(basePlatform);
    renderApp("/setup", backendWith({ getPlatformReport }));
    await openStep(/6\. Re-check/);
    expect(await screen.findByText(/Some things are still missing/)).toBeInTheDocument();
    const before = getPlatformReport.mock.calls.length;
    await userEvent.click(screen.getByRole("button", { name: "Re-check my computer" }));
    await waitFor(() => expect(getPlatformReport.mock.calls.length).toBeGreaterThan(before));
    await userEvent.click(screen.getByRole("button", { name: "Finish for now" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Dashboard" })).toBeInTheDocument();
    expect(localStorage.getItem(SETUP_SEEN_KEY)).toBe("1");
  });

  it("reports everything ready when the checks pass", async () => {
    const p = await basePlatform();
    const ready = { state: "ready" as const, summary: "ready", missing: [], blockers: [], notes: [] };
    p.readiness = { windows_tools: ready, linux_tools_wsl: ready, containers: ready };
    p.setup_actions = [];
    renderApp("/setup", backendWith({ getPlatformReport: async () => p }));
    await openStep(/6\. Re-check/);
    expect(await screen.findByText("Everything you chose is ready.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Finish" })).toBeInTheDocument();
  });

  it("shows a recoverable error when detection fails", async () => {
    renderApp("/setup", backendWith({ getPlatformReport: () => Promise.reject({ code: "environment", message: "probe failed", recovery: "Restart the app." }) }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Restart the app.");
  });
});

describe("blocked and unknown readiness on the dashboard", () => {
  it("shows blockers and unknowns without suggesting they are ready", async () => {
    const p = await basePlatform();
    p.readiness.linux_tools_wsl = { state: "blocked", summary: "Virtualization is turned off", missing: [], blockers: ["Hardware virtualization is disabled in firmware; WSL 2 cannot start."], notes: [] };
    p.readiness.containers = { state: "unknown", summary: "The container engine could not be checked: timed out", missing: [], blockers: [], notes: [] };
    renderApp("/", backendWith({ getPlatformReport: async () => p }));
    const list = await screen.findByRole("list", { name: "Environment checklist" });
    expect(within(list).getByText("Blocked")).toBeInTheDocument();
    expect(within(list).getByText("Unknown")).toBeInTheDocument();
    expect(within(list).getByText(/disabled in firmware/)).toBeInTheDocument();
  });
});

describe("launching container tools from Run a Tool", () => {
  const installed = (id: string) =>
    backendWith({ listToolStates: async () => [{ tool_id: id, state: "installed" satisfies InstallState }] });

  async function openRun(backend: Backend, profile: string, id = "semgrep") {
    renderApp(`/run/${id}?profile=${profile}`, backend);
    return screen.findByRole("button", { name: "Review what will run" });
  }

  it("blocks Launch with a reason for tools that have no launch profile", async () => {
    renderApp("/catalog/wireshark", installed("wireshark"));
    const launch = await screen.findByRole("button", { name: "Launch" });
    expect(launch).toBeDisabled();
    expect(screen.getByText(/no launch profile yet/)).toBeInTheDocument();
  });

  it("the Launch button on a tool page opens Run a Tool for that tool", async () => {
    renderApp("/catalog/semgrep", installed("semgrep"));
    await waitFor(async () => expect(await screen.findByRole("button", { name: "Launch" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Launch" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Run a Tool" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Review what will run" })).toBeInTheDocument();
  });

  it("requires a reviewed preview and an explicit approval before running, and shows what will run", async () => {
    const previewLaunch = vi.fn(createDemoBackend().previewLaunch);
    const launchTool = vi.fn(async () => "job-9");
    const reviewButton = await openRun({ ...installed("semgrep"), previewLaunch, launchTool }, "scan-local");
    const run = () => screen.queryByRole("button", { name: "Run" });
    expect(run()).not.toBeInTheDocument();
    // No folder yet: the review is refused with an actionable message.
    await userEvent.click(reviewButton);
    expect(await screen.findByRole("alert")).toHaveTextContent("choose and approve a folder");
    await userEvent.type(screen.getByLabelText("Folder for source"), "D:\labs\app");
    await userEvent.click(screen.getByRole("button", { name: "Review what will run" }));
    const summary = await screen.findByLabelText("What will run");
    expect(summary).toHaveTextContent("none (no internet access)");
    expect(summary).toHaveTextContent("D:\labs\app → /src (read-only)");
    expect(summary).toHaveTextContent("non-root user 65532:65532");
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    expect(screen.getByRole("button", { name: "Run" })).toBeEnabled();
    // Editing anything invalidates the review and the approval.
    await userEvent.type(screen.getByLabelText("Folder for source"), "x");
    expect(screen.queryByLabelText("What will run")).not.toBeInTheDocument();
    expect(run()).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Review what will run" }));
    await screen.findByLabelText("What will run");
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    await userEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() => expect(launchTool).toHaveBeenCalledTimes(1));
    expect(launchTool).toHaveBeenCalledWith("semgrep", "scan-local", expect.objectContaining({ config: "/src/.semgrep.yml" }), { network: false, mounts: { source: "D:\labs\appx" }, ports: {}, scope: null });
  });

  it("requires explicit network approval for profiles that need the internet and flags it", async () => {
    const launchTool = vi.fn(async () => "job-9");
    const reviewButton = await openRun({ ...installed("semgrep"), launchTool }, "scan-registry-rules");
    await userEvent.type(screen.getByLabelText("Folder for source"), "D:\labs\app");
    await userEvent.click(reviewButton);
    expect(await screen.findByRole("alert")).toHaveTextContent("network access");
    await userEvent.click(screen.getByRole("checkbox", { name: /Allow this run to use the network/ }));
    await userEvent.click(screen.getByRole("button", { name: "Review what will run" }));
    const summary = await screen.findByLabelText("What will run");
    expect(summary).toHaveTextContent("ALLOWED");
    expect(within(summary).getByText(/reach the internet/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    await userEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() => expect(launchTool).toHaveBeenCalledWith("semgrep", "scan-registry-rules", {}, { network: true, mounts: { source: "D:\labs\app" }, ports: {}, scope: null }));
  });

  it("shows backend errors (for example a refused folder) without starting anything", async () => {
    const launchTool = vi.fn(async () => "x");
    const previewLaunch = vi.fn(() => Promise.reject({ code: "validation", message: "validation failed: 'C:\Windows' is a protected system folder and cannot be mounted", recovery: "Correct the highlighted value and try again." }));
    const reviewButton = await openRun({ ...installed("semgrep"), previewLaunch, launchTool }, "scan-local");
    await userEvent.type(screen.getByLabelText("Folder for source"), "C:\Windows");
    await userEvent.click(reviewButton);
    expect(await screen.findByRole("alert")).toHaveTextContent("protected system folder");
    expect(screen.queryByRole("button", { name: "Run" })).not.toBeInTheDocument();
    expect(launchTool).not.toHaveBeenCalled();
  });
});

describe("install plan notes for Linux and container tools", () => {
  it("states that WSL is not a sandbox and where the tool will live", async () => {
    const p = await basePlatform();
    p.readiness.linux_tools_wsl = { state: "ready", summary: "ready", missing: [], blockers: [], notes: [] };
    renderApp("/catalog/nikto", backendWith({ getPlatformReport: async () => p, listToolStates: async () => [{ tool_id: "nikto", state: "not_installed" }] }));
    await waitFor(async () => expect(await screen.findByRole("button", { name: "Install" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText(/WSL is not a sandbox/)).toBeInTheDocument();
  });
});

describe("jobs for setup and runs", () => {
  it("shows a running tool and a setup job without a Cancel button", async () => {
    const now = new Date().toISOString();
    renderApp("/jobs", {
      ...createDemoBackend(),
      listJobs: async () => [
        { id: "r1", tool_id: "semgrep", kind: "run", phase: "running", message: "Running semgrep scan /src in a container", started_at: now, updated_at: now, allow_elevation: false },
        { id: "s1", tool_id: "setup-install-wsl", kind: "setup", phase: "installing", message: "Install WSL", started_at: now, updated_at: now, allow_elevation: true },
      ],
    });
    expect(await screen.findByRole("table")).toBeInTheDocument();
    expect(screen.getByText("Running")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Cancel Semgrep Run/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Cancel setup-install-wsl/ })).not.toBeInTheDocument();
  });
});
