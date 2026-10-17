import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { InstallState } from "@/domain/types";
import { createDemoBackend } from "@/infra/demoBackend";
import { unavailableBackend } from "@/infra/unavailableBackend";
import { readOnlyBackend, renderApp } from "@/test/render";

const err = { code: "environment", message: "probe failed", recovery: "Reinstall winget." };

function withStates(states: Record<string, InstallState | null>, overrides: Partial<Backend> = {}): Backend {
  const demo = createDemoBackend();
  return {
    ...demo,
    listToolStates: async () => Object.entries(states).map(([tool_id, state]) => ({ tool_id, state })),
    ...overrides,
  };
}

describe("navigation", () => {
  it("shows all seven sidebar destinations", async () => {
    renderApp("/");
    const nav = within(screen.getByRole("navigation", { name: "Main" }));
    for (const name of ["Dashboard", "Tool Catalog", "Installed Tools", "Running Jobs", "Learning Labs", "Activity", "Settings"]) {
      expect(nav.getByRole("link", { name })).toBeInTheDocument();
    }
  });

  it.each([
    ["/installed", "Installed Tools"],
    ["/jobs", "Running Jobs"],
    ["/labs", "Learning Labs"],
    ["/activity", "Activity"],
    ["/settings", "Settings"],
  ])("renders %s", async (path, heading) => {
    renderApp(path);
    expect(await screen.findByRole("heading", { level: 1, name: heading })).toBeInTheDocument();
  });
});

describe("demo labelling", () => {
  it("shows a demonstration banner only for the demo backend", async () => {
    const { unmount } = renderApp("/");
    expect(await screen.findByText(/Demonstration data/)).toBeInTheDocument();
    unmount();
    renderApp("/", readOnlyBackend());
    await screen.findByRole("heading", { level: 1, name: "Dashboard" });
    expect(screen.queryByText(/Demonstration data/)).not.toBeInTheDocument();
  });
});

describe("dashboard", () => {
  it("shows the environment checklist with recovery steps and recommended tools", async () => {
    renderApp("/");
    const list = await screen.findByRole("list", { name: "Environment checklist" });
    expect(within(list).getByText("WSL 2")).toBeInTheDocument();
    expect(screen.getByText(/wsl --install/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Browse tools" })).toHaveAttribute("href", "/catalog");
    expect(await screen.findByRole("link", { name: /Wireshark/ })).toBeInTheDocument();
  });

  it("does not invent counts when the backend cannot detect them", async () => {
    renderApp("/", readOnlyBackend());
    expect(await screen.findByText("Installed-tool detection is not available yet.")).toBeInTheDocument();
    expect(screen.getByText("Job tracking is not available yet.")).toBeInTheDocument();
  });

  it("shows a recoverable error when environment detection fails", async () => {
    renderApp("/", readOnlyBackend({ detectEnvironment: () => Promise.reject(err) }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("probe failed");
    expect(alert).toHaveTextContent("Reinstall winget.");
  });

  it("explains how to start the backend when none is available", async () => {
    renderApp("/catalog", unavailableBackend);
    const alerts = await screen.findAllByRole("alert");
    expect(alerts[0]).toHaveTextContent("backend is not running");
  });
});

describe("catalog", () => {
  it("lists the seeded tools with status badges", async () => {
    renderApp("/catalog");
    for (const n of ["Nmap", "OWASP ZAP", "Wireshark", "Semgrep", "Trivy"]) {
      expect(await screen.findByRole("heading", { name: n, level: 3 })).toBeInTheDocument();
    }
    expect(screen.getByText("5 of 5 tools")).toBeInTheDocument();
  });

  it("searches name and description", async () => {
    renderApp("/catalog");
    await screen.findByRole("heading", { name: "Nmap", level: 3 });
    await userEvent.type(screen.getByLabelText("Search tools"), "proxy");
    await waitFor(() => expect(screen.getByText("1 of 5 tools")).toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "OWASP ZAP", level: 3 })).toBeInTheDocument();
  });

  it("filters by category, platform, method and status, then clears", async () => {
    renderApp("/catalog");
    await screen.findByRole("heading", { name: "Nmap", level: 3 });
    await userEvent.selectOptions(screen.getByLabelText("Category"), "vulnerability");
    expect(screen.getByText("2 of 5 tools")).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("Platform"), "container");
    expect(screen.getByText("1 of 5 tools")).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("Execution method"), "winget");
    expect(await screen.findByText("No tools match your filters")).toBeInTheDocument();
    await userEvent.click(screen.getAllByRole("button", { name: "Clear filters" })[0]!);
    expect(await screen.findByText("5 of 5 tools")).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("Status"), "verification_failed");
    expect(screen.getByText("1 of 5 tools")).toBeInTheDocument();
  });

  it("sorts by name descending", async () => {
    renderApp("/catalog");
    await screen.findByRole("heading", { name: "Nmap", level: 3 });
    await userEvent.selectOptions(screen.getByLabelText("Sort by"), "name_desc");
    const first = screen.getAllByRole("heading", { level: 3 })[0];
    expect(first).toHaveTextContent("Wireshark");
  });

  it("shows an empty state for an empty catalog and an error with retry on failure", async () => {
    const { unmount } = renderApp("/catalog", readOnlyBackend({ listTools: async () => [] }));
    expect(await screen.findByText("The catalog is empty")).toBeInTheDocument();
    unmount();
    renderApp("/catalog", readOnlyBackend({ listTools: () => Promise.reject(err) }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Reinstall winget.");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });
});

describe("status rendering", () => {
  const cases: [InstallState | null, string][] = [
    [null, "Status unknown"],
    ["not_installed", "Ready to install"],
    ["installing", "Installing"],
    ["installed", "Installed"],
    ["verification_failed", "Verification failed"],
    ["update_available", "Update available"],
    ["uninstalling", "Uninstalling"],
    ["failed", "Error"],
  ];
  it.each(cases)("renders %s as '%s' on the tool page", async (state, label) => {
    renderApp("/catalog/nmap", withStates({ nmap: state }));
    const heading = await screen.findByRole("heading", { name: "Nmap", level: 1 });
    await waitFor(() => expect(within(heading.parentElement!).getByText(label)).toBeInTheDocument());
  });

  it("shows 'Not installed' (not ready) when a prerequisite is missing", async () => {
    renderApp("/catalog/semgrep", withStates({ semgrep: "not_installed" }));
    const heading = await screen.findByRole("heading", { name: "Semgrep", level: 1 });
    await waitFor(() => expect(within(heading.parentElement!).getByText("Not installed")).toBeInTheDocument());
  });
});

describe("tool details and actions", () => {
  it("shows prerequisites, runtimes, license, maintenance and links", async () => {
    renderApp("/catalog/semgrep");
    expect(await screen.findByRole("heading", { name: "Semgrep", level: 1 })).toBeInTheDocument();
    expect(screen.getByText("Container")).toBeInTheDocument();
    expect(await screen.findByText(/Docker · missing/)).toBeInTheDocument();
    expect(screen.getByText("LGPL-2.1-only")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Official documentation/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Source repository/ })).toBeInTheDocument();
  });

  it("marks actions unavailable, with a reason, when the backend lacks the capability", async () => {
    renderApp("/catalog/nmap", readOnlyBackend({ listToolStates: async () => [{ tool_id: "nmap", state: "not_installed" }] }));
    const install = await screen.findByRole("button", { name: "Install" });
    expect(install).toBeDisabled();
    expect(screen.getByText(/Not available in this version/)).toBeInTheDocument();
  });

  it("requires confirmation before installing and then calls the backend", async () => {
    const installTool = vi.fn(async () => "op-1");
    renderApp("/catalog/trivy", withStates({ trivy: "not_installed" }, { installTool }));
    await waitFor(async () => expect(await screen.findByRole("button", { name: "Install" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    expect(installTool).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog", { name: "Install Trivy?" });
    expect(within(dialog).getByText(/AquaSecurity\.Trivy/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Install" }));
    await waitFor(() => expect(installTool).toHaveBeenCalledWith("trivy", false));
  });

  it("requires explicit elevation consent for tools that need administrator rights", async () => {
    const installTool = vi.fn(async () => "op-1");
    renderApp("/catalog/wireshark", withStates({ wireshark: "not_installed" }, { installTool }));
    await waitFor(async () => expect(await screen.findByRole("button", { name: "Install" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    const dialog = await screen.findByRole("dialog");
    const confirm = within(dialog).getByRole("button", { name: "Install" });
    expect(confirm).toBeDisabled();
    await userEvent.click(within(dialog).getByRole("checkbox"));
    await userEvent.click(confirm);
    await waitFor(() => expect(installTool).toHaveBeenCalledWith("wireshark", true));
  });

  it("shows the backend error and recovery text when an action fails", async () => {
    const installTool = vi.fn(() => Promise.reject({ code: "not_implemented", message: "'install' is not implemented", recovery: "Planned for a later phase." }));
    renderApp("/catalog/trivy", withStates({ trivy: "not_installed" }, { installTool }));
    await waitFor(async () => expect(await screen.findByRole("button", { name: "Install" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Install" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Planned for a later phase.");
  });

  it("confirms before uninstalling", async () => {
    const uninstallTool = vi.fn(async () => "op-2");
    renderApp("/catalog/nmap", withStates({ nmap: "installed" }, { uninstallTool }));
    await waitFor(async () => expect(await screen.findByRole("button", { name: "Uninstall" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Uninstall" }));
    expect(uninstallTool).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog", { name: "Uninstall Nmap?" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Uninstall" }));
    await waitFor(() => expect(uninstallTool).toHaveBeenCalledWith("nmap", false));
  });

  it("opens links through the backend rather than navigating", async () => {
    const openLink = vi.fn(async () => undefined);
    renderApp("/catalog/nmap", { ...createDemoBackend(), openLink });
    await userEvent.click(await screen.findByRole("button", { name: /Official documentation/ }));
    expect(openLink).toHaveBeenCalledWith("nmap", "documentation");
  });

  it("shows an error for an unknown tool", async () => {
    renderApp("/catalog/does-not-exist");
    expect(await screen.findByRole("alert")).toHaveTextContent("was not found");
  });
});

describe("installed tools", () => {
  it("shows version, location, environment and verification result", async () => {
    renderApp("/installed");
    expect(await screen.findByText("Version 7.95 (demo)")).toBeInTheDocument();
    expect(screen.getByText("C:\\Demo\\Tools\\nmap")).toBeInTheDocument();
    expect(screen.getAllByText("Windows").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Passed")).toHaveLength(2);
    expect(screen.getAllByText("Failed")).toHaveLength(1);
  });

  it("does not claim nothing is installed when detection is unavailable", async () => {
    renderApp("/installed", readOnlyBackend());
    expect(await screen.findByText("Installed-tool detection is not available yet")).toBeInTheDocument();
  });

  it("shows a plain empty state when detection works and nothing is installed", async () => {
    renderApp("/installed", readOnlyBackend({ getCapabilities: async () => ({ ...(await readOnlyBackend().getCapabilities()), installed_detection: true }) }));
    expect(await screen.findByText("No tools installed")).toBeInTheDocument();
  });
});

describe("jobs", () => {
  it("lists jobs with state, exit status, output and cancel", async () => {
    renderApp("/jobs");
    expect(await screen.findByRole("table")).toBeInTheDocument();
    expect(screen.getByText("Running")).toBeInTheDocument();
    expect(screen.getByText("Succeeded")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /View output for Nmap/ }));
    expect(await screen.findByLabelText("Job output")).toHaveTextContent("[demo] Installing nmap");
  });

  it("asks for confirmation before cancelling", async () => {
    const cancelJob = vi.fn(async () => undefined);
    renderApp("/jobs", { ...createDemoBackend(), cancelJob });
    await userEvent.click(await screen.findByRole("button", { name: /Cancel Wireshark/ }));
    expect(cancelJob).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel job" }));
    await waitFor(() => expect(cancelJob).toHaveBeenCalledWith("demo-job-3"));
  });

  it("explains an empty list when job execution is unavailable", async () => {
    renderApp("/jobs", readOnlyBackend());
    expect(await screen.findByText("No jobs yet")).toBeInTheDocument();
    expect(screen.getByText(/not available in this version/)).toBeInTheDocument();
  });
});

describe("labs and activity", () => {
  it("shows an honest empty state for labs", async () => {
    renderApp("/labs", readOnlyBackend());
    expect(await screen.findByText("No labs available yet")).toBeInTheDocument();
  });

  it("disables lab start when unsupported", async () => {
    renderApp("/labs");
    expect(await screen.findByRole("button", { name: "Start lab" })).toBeDisabled();
  });

  it("lists activity and handles an empty log", async () => {
    const { unmount } = renderApp("/activity");
    expect(await screen.findByText("Demo: installed Nmap")).toBeInTheDocument();
    unmount();
    renderApp("/activity", readOnlyBackend());
    expect(await screen.findByText("No activity yet")).toBeInTheDocument();
  });
});
