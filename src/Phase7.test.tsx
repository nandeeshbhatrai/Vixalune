import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { InstallState } from "@/domain/types";
import { SETUP_SEEN_KEY } from "@/pages/SetupWizard";
import { createDemoBackend } from "@/infra/demoBackend";
import { renderApp } from "@/test/render";

function backendWith(overrides: Partial<Backend> = {}): Backend {
  return { ...createDemoBackend(), listJobs: async () => [], ...overrides };
}

describe("first-run welcome tour", () => {
  beforeEach(() => localStorage.clear());

  it("opens once on first launch and changes nothing by itself", async () => {
    const installTool = vi.fn(async () => "x");
    const runSetupAction = vi.fn(async () => "x");
    const updateStorage = vi.fn();
    renderApp("/", backendWith({ installTool, runSetupAction, updateStorage }), { firstRunRedirect: true });
    expect(await screen.findByRole("heading", { level: 1, name: "Welcome to Vixalune" })).toBeInTheDocument();
    expect(installTool).not.toHaveBeenCalled();
    expect(runSetupAction).not.toHaveBeenCalled();
    expect(updateStorage).not.toHaveBeenCalled();
  });

  it("explains the application and the supported environments, and says the optional runtimes are optional", async () => {
    renderApp("/welcome", backendWith());
    const s = await screen.findByRole("region", { name: "Welcome" });
    expect(s).toHaveTextContent("authorized testing and learning only");
    expect(s).toHaveTextContent("Windows 11, or Windows 10 version 22H2");
    expect(s).toHaveTextContent("Both are optional");
  });

  it("shows readiness checks with optional runtimes marked optional and points to the detailed setup guide", async () => {
    renderApp("/welcome", backendWith());
    await userEvent.click(await screen.findByRole("button", { name: /2\. Your computer/ }));
    const list = await screen.findByRole("list", { name: "Readiness" });
    await waitFor(() => expect(within(list).getAllByText(/Ready|Needs setup|Blocked|Unknown/).length).toBeGreaterThan(0));
    expect(within(list).getAllByText("Optional")).toHaveLength(2);
    expect(screen.getByRole("link", { name: "Setup guide" })).toHaveAttribute("href", "/setup");
  });

  it("lets the user change storage locations and says they apply after a restart; invalid paths are refused", async () => {
    const base = backendWith();
    const updateStorage = vi.fn(base.updateStorage);
    renderApp("/welcome", { ...base, updateStorage });
    await userEvent.click(await screen.findByRole("button", { name: /3\. Where things are stored/ }));
    const labs = await screen.findByLabelText("Practice labs folder");
    await userEvent.clear(labs);
    await userEvent.type(labs, "relative\\path");
    await userEvent.click(screen.getByRole("button", { name: "Save locations" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("full folder path");
    await userEvent.clear(labs);
    await userEvent.type(labs, "D:\\Vixalune\\labs");
    await userEvent.click(screen.getByRole("button", { name: "Save locations" }));
    await waitFor(() => expect(updateStorage).toHaveBeenLastCalledWith(expect.objectContaining({ labs_dir: "D:\\Vixalune\\labs" })));
    expect(await screen.findByText(/used the next time you start Vixalune/)).toBeInTheDocument();
    expect(screen.getByText(/kept separate from the program files/)).toBeInTheDocument();
  });

  it("explains permission prompts and the network-scope safeguards", async () => {
    renderApp("/welcome", backendWith());
    await userEvent.click(await screen.findByRole("button", { name: /4\. Permissions and safety/ }));
    const s = await screen.findByRole("region", { name: "Permissions and safety" });
    expect(s).toHaveTextContent("does not ask for administrator rights itself");
    expect(s).toHaveTextContent("administrator prompt (UAC)");
    expect(s).toHaveTextContent("never changes security settings");
    expect(s).toHaveTextContent("type its name");
    expect(s).toHaveTextContent("cannot tell whether you are allowed");
  });

  it("shows the curated catalog, then a few starter tools whose Install button opens the tool page", async () => {
    renderApp("/welcome", backendWith({ listToolStates: async () => [{ tool_id: "nmap", state: "not_installed" satisfies InstallState }] }));
    await userEvent.click(await screen.findByRole("button", { name: /5\. The catalog/ }));
    const catalog = await screen.findByRole("list", { name: "Catalog preview" });
    expect(within(catalog).getAllByRole("listitem").length).toBe(6);
    await userEvent.click(screen.getByRole("button", { name: /6\. Your first tools/ }));
    const starters = await screen.findByRole("list", { name: "Starter tools" });
    expect(within(starters).getAllByRole("listitem").length).toBeLessThanOrEqual(3);
    await userEvent.click(within(starters).getByRole("button", { name: /Install Nmap/ }));
    expect(await screen.findByRole("heading", { level: 1, name: "Nmap" })).toBeInTheDocument();
    expect(localStorage.getItem(SETUP_SEEN_KEY)).toBe("1");
  });

  it("can be skipped, and is not shown again afterwards", async () => {
    renderApp("/", backendWith(), { firstRunRedirect: true });
    await userEvent.click(await screen.findByRole("button", { name: "Skip the tour" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Dashboard" })).toBeInTheDocument();
    expect(localStorage.getItem(SETUP_SEEN_KEY)).toBe("1");
  });

  it("works when WSL and Docker are missing: the tour still reaches the catalog", async () => {
    // The demo platform already reports WSL missing and the container engine stopped.
    renderApp("/welcome", backendWith());
    await userEvent.click(await screen.findByRole("button", { name: /5\. The catalog/ }));
    expect(await screen.findByRole("list", { name: "Catalog preview" })).toBeInTheDocument();
  });
});

describe("version, release notes and updates", () => {
  it("shows the version, release notes and update guidance in Settings, and opens the release page through the backend", async () => {
    const openReleasePage = vi.fn(async () => undefined);
    renderApp("/settings", backendWith({ openReleasePage }));
    expect(await screen.findByText(/You are running/)).toHaveTextContent("version demo");
    expect(screen.getByText(/does not check for, download or run updates by itself/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show release notes" }));
    expect(screen.getByLabelText("Release notes")).toHaveTextContent("0.2.0");
    await userEvent.click(screen.getByRole("button", { name: "Open the release page" }));
    expect(openReleasePage).toHaveBeenCalledTimes(1);
  });

  it("lists catalog definitions with their versions and says remote catalogs are off; surfaces tool updates", async () => {
    renderApp("/settings", backendWith());
    const table = await screen.findByRole("table", { name: "Catalog entries" });
    expect(within(table).getByText("Nmap")).toBeInTheDocument();
    expect(within(table).getAllByText("bundled with Vixalune").length).toBeGreaterThan(0);
    expect(screen.getByText(/Remote catalogs are disabled/)).toBeInTheDocument();
    expect(await screen.findByText(/1 tool update is available: Wireshark/)).toBeInTheDocument();
  });

  it("shows the version in the sidebar", async () => {
    renderApp("/", backendWith());
    expect(await screen.findByText("Version demo")).toBeInTheDocument();
  });
});
