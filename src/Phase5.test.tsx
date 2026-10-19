import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Backend } from "@/domain/backend";
import type { InstallState, Job, OutputRow, RunOutcome, RunRecord } from "@/domain/types";
import { createDemoBackend } from "@/infra/demoBackend";
import { renderApp } from "@/test/render";

const LONG = { timeout: 6000 };

function backendWith(overrides: Partial<Backend> = {}): Backend {
  const base = createDemoBackend();
  return { ...base, listJobs: async () => [], listToolStates: async () => ["nmap", "zap", "trivy", "semgrep"].map((tool_id) => ({ tool_id, state: "installed" satisfies InstallState })), ...overrides };
}

async function review() {
  await userEvent.click(await screen.findByRole("button", { name: "Review what will run" }));
  return screen.findByLabelText("What will run");
}

describe("Run a Tool: guided parameters and preview", () => {
  it("starts from the safe local lab defaults and shows the exact command, scope and limits before anything runs", async () => {
    const launchTool = vi.fn(async () => "j");
    renderApp("/run/nmap", backendWith({ launchTool }));
    const target = await screen.findByLabelText(/Target/);
    await waitFor(() => expect(target).toHaveValue("127.0.0.1"));
    const preview = await review();
    expect(within(preview).getByLabelText("Command preview")).toHaveTextContent("nmap.exe 127.0.0.1");
    expect(preview).toHaveTextContent("Sends traffic to the targets you named");
    expect(preview).toHaveTextContent("Time limit: 600 seconds");
    expect(preview).toHaveTextContent("never asks for administrator rights");
    const scope = screen.getByLabelText("Target and scope");
    expect(scope).toHaveTextContent("This computer");
    expect(scope).toHaveTextContent("not proof that you are authorised");
    expect(launchTool).not.toHaveBeenCalled();
    expect(screen.queryByRole("group", { name: "Confirm the target" })).not.toBeInTheDocument();
  });

  it("needs the review tick-box before Run is enabled, and any edit voids the review", async () => {
    renderApp("/run/nmap", backendWith());
    await review();
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    expect(screen.getByRole("button", { name: "Run" })).toBeEnabled();
    await userEvent.type(screen.getByRole("textbox", { name: /^Target \(required\)/ }), "x");
    expect(screen.queryByRole("button", { name: "Run" })).not.toBeInTheDocument();
  });

  it("shows validation errors from the backend for malformed values and does not offer Run", async () => {
    renderApp("/run/nmap", backendWith());
    const target = await screen.findByLabelText(/Target/);
    await waitFor(() => expect(target).toHaveValue("127.0.0.1"));
    await userEvent.clear(target);
    await userEvent.type(target, "127.0.0.1; calc.exe");
    await userEvent.click(screen.getByRole("button", { name: "Review what will run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("not a valid host name or IP address");
    expect(screen.queryByRole("button", { name: "Run" })).not.toBeInTheDocument();
  });

  it("uses the profile chosen in a tutorial link", async () => {
    renderApp("/run/nmap?profile=service-scan", backendWith());
    const ports = await screen.findByLabelText(/Ports/);
    await waitFor(() => expect(ports).toHaveValue("18080"));
  });

  it("warns, without blocking the review, when the tool is not installed", async () => {
    renderApp("/run/nmap", backendWith({ listToolStates: async () => [{ tool_id: "nmap", state: "not_installed" }] }));
    expect(await screen.findByText(/does not show as installed/)).toBeInTheDocument();
  });
});

describe("Run a Tool: scope and confirmation", () => {
  async function typeTarget(value: string) {
    const target = await screen.findByLabelText(/Target/);
    await waitFor(() => expect(target).toHaveValue("127.0.0.1"));
    await userEvent.clear(target);
    await userEvent.type(target, value);
  }

  it("requires a tick and the typed host name for a public target, and sends exactly what was confirmed", async () => {
    const launchTool = vi.fn(async () => "j");
    renderApp("/run/nmap", backendWith({ launchTool }));
    await typeTarget("scanme.example.org");
    await review();
    const group = await screen.findByRole("group", { name: "Confirm the target" });
    expect(screen.getByLabelText("Target and scope")).toHaveTextContent("Public internet");
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
    await userEvent.click(within(group).getByRole("checkbox"));
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
    await userEvent.type(within(group).getByLabelText("Type the target to confirm"), "wrong.example");
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
    await userEvent.clear(within(group).getByLabelText("Type the target to confirm"));
    await userEvent.type(within(group).getByLabelText("Type the target to confirm"), "scanme.example.org");
    expect(screen.getByRole("button", { name: "Run" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() => expect(launchTool).toHaveBeenCalledTimes(1));
    expect(launchTool).toHaveBeenCalledWith("nmap", "quick-scan", { target: "scanme.example.org" }, expect.objectContaining({ scope: { acknowledged: true, typed: "scanme.example.org", addresses: ["93.184.216.34"] } }));
  });

  it("only needs a tick for a private network address that is not in the lab list", async () => {
    renderApp("/run/nmap", backendWith());
    await typeTarget("192.168.1.20");
    await review();
    const group = await screen.findByRole("group", { name: "Confirm the target" });
    expect(within(group).queryByLabelText("Type the target to confirm")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    await userEvent.click(within(group).getByRole("checkbox"));
    expect(screen.getByRole("button", { name: "Run" })).toBeEnabled();
  });

  it("refuses blocked targets at review time", async () => {
    renderApp("/run/nmap", backendWith());
    await typeTarget("0.0.0.0");
    await userEvent.click(screen.getByRole("button", { name: "Review what will run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("out of scope");
  });

  it("the backend, not the page, has the last word: a refused start shows its reason and no output panel", async () => {
    const launchTool = vi.fn(() => Promise.reject({ code: "validation", message: "validation failed: the addresses this target resolves to changed since you reviewed them; review and confirm again", recovery: "Correct the highlighted value and try again." }));
    renderApp("/run/nmap", backendWith({ launchTool }));
    await review();
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    await userEvent.click(screen.getByRole("button", { name: "Run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("changed since you reviewed");
    expect(screen.queryByLabelText("Run output")).not.toBeInTheDocument();
  });
});

describe("Run a Tool: running, output and history", () => {
  async function startDefaultRun(backend: Backend) {
    renderApp("/run/nmap", backend);
    await review();
    await userEvent.click(screen.getByRole("checkbox", { name: /reviewed the command and target/ }));
    await userEvent.click(screen.getByRole("button", { name: "Run" }));
    return screen.findByLabelText("Run output");
  }

  it("streams output live and shows a run with findings as a result, not an error", async () => {
    const panel = await startDefaultRun(backendWith());
    expect(await within(panel).findByText(/Nmap done/, undefined, LONG)).toBeInTheDocument();
    expect(await within(panel).findByText("Completed with findings", undefined, LONG)).toBeInTheDocument();
    expect(within(panel).getByText(/This is not an error/)).toBeInTheDocument();
    expect(within(panel).queryByRole("alert")).not.toBeInTheDocument();
    expect(within(panel).getByText("Exit code:").parentElement).toHaveTextContent("0");
    expect(within(panel).getByText("Scope:").parentElement).toHaveTextContent("this computer (loopback)");
    // The same run appears in the history with the same distinction.
    const history = await screen.findByRole("region", { name: "Recent runs" });
    expect(await within(history).findByText("Completed with findings", undefined, LONG)).toBeInTheDocument();
  });

  it("shows a failed run differently, with the reason and the exit code", async () => {
    const base = backendWith();
    let polls = 0;
    const job: Job = { id: "f1", tool_id: "nmap", kind: "run", phase: "failed", message: "the tool exited with code 2", started_at: new Date().toISOString(), updated_at: new Date().toISOString(), finished_at: new Date().toISOString(), exit_code: 2, allow_elevation: false, error: { code: "command_failed", message: "the tool exited with code 2, which this profile does not treat as success", recovery: "Check the output below.", detail: "QUITTING!" } };
    const run: RunRecord = { id: "f1", tool_id: "nmap", tool_name: "Nmap", profile_id: "quick-scan", profile_label: "Scan", started_at: job.started_at, finished_at: job.finished_at, duration_ms: 1500, exit_code: 2, outcome: "failed" satisfies RunOutcome, command: "nmap.exe -F --open nope", targets: [], scope: "no network targets", network_activity: "probes_targets", output_format: "text" };
    const panel = await startDefaultRun({ ...base, launchTool: async () => "f1", getJob: async () => job, getRun: async () => run, getRunOutput: async () => (polls++ === 0 ? [{ seq: 1, stream: "stderr", text: "QUITTING!" }] : []) });
    expect((await within(panel).findAllByText("Failed")).length).toBeGreaterThan(0);
    expect(within(panel).getByRole("alert")).toHaveTextContent("does not treat as success");
    expect(within(panel).getByText("Exit code:").parentElement).toHaveTextContent("2");
    expect(within(panel).queryByText("Completed with findings")).not.toBeInTheDocument();
  });

  it("can be cancelled after confirmation, and a cancelled run is labelled as cancelled", { timeout: 15000 }, async () => {
    const base = backendWith();
    const cancelJob = vi.fn(base.cancelJob);
    const panel = await startDefaultRun({ ...base, cancelJob });
    await userEvent.click(await within(panel).findByRole("button", { name: /Cancel run/ }));
    const dialog = await screen.findByRole("dialog", { name: "Cancel this run?" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel run" }));
    await waitFor(() => expect(cancelJob).toHaveBeenCalledTimes(1));
    expect((await within(panel).findAllByText("Cancelled", undefined, LONG)).length).toBeGreaterThan(0);
    expect(within(panel).queryByRole("button", { name: /Cancel run/ })).not.toBeInTheDocument();
  });

  it("searches, copies and saves output; saving goes through the backend dialog", async () => {
    const write = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    const base = backendWith();
    const saveRunOutput = vi.fn(base.saveRunOutput);
    const panel = await startDefaultRun({ ...base, saveRunOutput });
    await within(panel).findByText("Completed with findings", undefined, LONG);
    await userEvent.type(within(panel).getByLabelText("Search output"), "open");
    expect(within(panel).getByText(/2 of \d+ lines match/)).toBeInTheDocument();
    await userEvent.click(within(panel).getByRole("button", { name: /Copy/ }));
    await waitFor(() => expect(write).toHaveBeenCalledTimes(1));
    expect(String((write.mock.calls[0] as unknown[])[0]).split("\n")).toHaveLength(2);
    await userEvent.click(within(panel).getByRole("button", { name: /Save output/ }));
    await waitFor(() => expect(saveRunOutput).toHaveBeenCalledTimes(1));
    expect(await within(panel).findByText(/Saved to C:\\Demo\\Documents/)).toBeInTheDocument();
    // The page never passes a path or text to the backend.
    expect((saveRunOutput.mock.calls[0] as unknown[]).length).toBe(1);
  });

  it("only offers Save when the run has finished", async () => {
    const base = backendWith();
    const job: Job = { id: "r1", tool_id: "nmap", kind: "run", phase: "running", message: "Running", started_at: new Date().toISOString(), updated_at: new Date().toISOString(), allow_elevation: false };
    const rows: OutputRow[] = [{ seq: 1, stream: "stdout", text: "scanning…" }];
    const panel = await startDefaultRun({ ...base, launchTool: async () => "r1", getJob: async () => job, getRunOutput: async (_id, after) => rows.filter((r) => r.seq > after) });
    expect(await within(panel).findByText("scanning…")).toBeInTheDocument();
    expect(within(panel).getByRole("button", { name: /Save output/ })).toBeDisabled();
    expect(within(panel).getByRole("button", { name: /Cancel run/ })).toBeInTheDocument();
  });
});

describe("Learn", () => {
  it("lists tutorials and shows the authorization boundary before anything else", async () => {
    renderApp("/learn", backendWith());
    expect(await screen.findByText(/Learn Nmap/)).toBeInTheDocument();
    await userEvent.click(screen.getAllByRole("link", { name: "Start the tutorial" })[0]!);
    expect(await screen.findByRole("note", { name: "Authorization boundary" })).toHaveTextContent("written permission");
    expect(screen.getByText("Your first run")).toBeInTheDocument();
    expect(screen.getByText("Example output and how to read it")).toBeInTheDocument();
    expect(screen.getByText("Common problems")).toBeInTheDocument();
    expect(screen.getByText("Practice exercise")).toBeInTheDocument();
    const note = screen.getByRole("note", { name: "Authorization boundary" });
    const first = screen.getByText("What it does");
    expect(note.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("opens official links through the backend by position, never by address", async () => {
    const base = backendWith();
    const openTutorialLink = vi.fn(async () => undefined);
    renderApp("/learn/nmap", { ...base, openTutorialLink });
    await userEvent.click(await screen.findByRole("button", { name: /documentation \(demo link\)/ }));
    expect(openTutorialLink).toHaveBeenCalledWith("nmap", 0);
  });

  it("explains a missing tutorial", async () => {
    renderApp("/learn/wireshark", backendWith());
    expect(await screen.findByRole("alert")).toHaveTextContent("tutorial");
  });
});

describe("Settings: lab list and retention", () => {
  it("saves a lab list and retention periods, and shows backend validation errors", async () => {
    const base = backendWith();
    const updateSettings = vi.fn(base.updateSettings);
    renderApp("/settings", { ...base, updateSettings });
    const labs = await screen.findByLabelText("Lab targets");
    await userEvent.type(labs, "192.168.56.0/24");
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => expect(updateSettings).toHaveBeenCalledWith({ scope: { lab_targets: ["192.168.56.0/24"] }, retention: { history_days: 90, output_days: 14 } }));
    expect(await screen.findByText(/Settings saved/)).toBeInTheDocument();
    await userEvent.clear(labs);
    await userEvent.type(labs, "0.0.0.0/0");
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("too wide");
    await userEvent.clear(labs);
    const output = screen.getByLabelText(/Keep run output for/);
    await userEvent.clear(output);
    await userEvent.type(output, "400");
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("output cannot be kept longer");
  });

  it("deletes history only after confirmation", async () => {
    const base = backendWith();
    const purgeHistoryNow = vi.fn(base.purgeHistoryNow);
    renderApp("/settings", { ...base, purgeHistoryNow });
    await userEvent.click(await screen.findByRole("button", { name: /Delete history and output now/ }));
    expect(purgeHistoryNow).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(purgeHistoryNow).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/Deleted 0 run record/)).toBeInTheDocument();
  });

  it("a lab list entry makes a private target need no confirmation in the demo scope check", async () => {
    const base = backendWith();
    await base.updateSettings({ scope: { lab_targets: ["192.168.56.10"] }, retention: { history_days: 90, output_days: 14 } });
    renderApp("/run/nmap", base);
    const target = await screen.findByLabelText(/Target/);
    await waitFor(() => expect(target).toHaveValue("127.0.0.1"));
    await userEvent.clear(target);
    await userEvent.type(target, "192.168.56.10");
    await review();
    expect(screen.getByLabelText("Target and scope")).toHaveTextContent("Your lab");
    expect(screen.queryByRole("group", { name: "Confirm the target" })).not.toBeInTheDocument();
  });
});
