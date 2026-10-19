// Drives the INSTALLED application through its real WebView2 window using the Chrome DevTools Protocol.
// Usage: node scripts/drive-installed.mjs <scenario> [outDir]
// The application must have been started with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222.
import { createRequire } from "node:module";
import { mkdirSync, writeFileSync } from "node:fs";
const require = createRequire(new URL("../package.json", import.meta.url));
const { chromium } = require("@playwright/test");

const scenario = process.argv[2] ?? "first-run";
const out = process.argv[3] ?? ".";
mkdirSync(out, { recursive: true });
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  -  " + detail : ""}`);
};

async function connect() {
  for (let i = 0; i < 60; i++) {
    try {
      const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
      const page = browser.contexts()[0]?.pages()[0];
      if (page) return { browser, page };
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("could not connect to the application's window");
}

const { browser, page } = await connect();
await page.waitForLoadState("domcontentloaded");
console.log("window url:", page.url(), "title:", await page.title());

const invoke = (cmd, args = {}) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);

try {
  if (scenario === "first-run") {
    if (process.env.RESET_FIRST_RUN) {
      // Re-run support: forget that the tour was seen (the first genuine launch was verified separately).
      await page.evaluate(() => localStorage.clear());
      await page.goto("http://tauri.localhost/#/");
      await page.reload();
    }
    await page.getByRole("heading", { level: 1, name: "Welcome to Vixalune" }).waitFor({ timeout: 20000 });
    check("first launch opens the welcome tour", true, page.url());
    check("not the demo backend (no demo banner)", (await page.getByText("Demonstration data").count()) === 0);
    await page.screenshot({ path: `${out}/01-welcome.png` });

    await page.getByRole("button", { name: /2\. Your computer/ }).click();
    const list = page.getByRole("list", { name: "Readiness" });
    await list.waitFor();
    await page.waitForFunction(() => !document.querySelector('[aria-label="Readiness"]')?.textContent?.includes("Checking"), null, { timeout: 60000 });
    const readiness = (await list.innerText()).replace(/\s+/g, " ");
    check("readiness checks ran against this computer", /Windows tools/.test(readiness) && !/Checking/.test(readiness), readiness.slice(0, 300));
    await page.screenshot({ path: `${out}/02-readiness.png` });

    await page.getByRole("button", { name: /3\. Where things are stored/ }).click();
    const tools = page.getByLabel("Tools folder");
    await tools.waitFor();
    const info = await invoke("get_storage_info");
    check("data folder is the one chosen by VIXALUNE_DATA_DIR", info.data_overridden !== false && info.data_dir_overridden === true, info.data_dir);
    const labs = page.getByLabel("Practice labs folder");
    await labs.fill("relative\\nope");
    await page.getByRole("button", { name: "Save locations" }).click();
    check("an invalid storage path is refused", await page.getByRole("alert").filter({ hasText: "full folder path" }).isVisible());
    await labs.fill(info.data_dir.replace(/\\data$/, "") + "\\labs-in-appdata");
    await page.getByRole("button", { name: "Save locations" }).click();
    check("a labs folder inside AppData is refused with an explanation", await page.getByRole("alert").filter({ hasText: "outside AppData" }).isVisible());
    const newLabs = process.env.TEST_LABS_DIR;
    await labs.fill(newLabs);
    await page.getByRole("button", { name: "Save locations" }).click();
    await page.getByText(/used the next time you start Vixalune/).waitFor();
    const after = await invoke("get_storage_info");
    check("storage change is saved and flagged for restart", after.labs_dir === newLabs && after.restart_required === true, after.labs_dir);
    await page.screenshot({ path: `${out}/03-storage.png` });

    await page.getByRole("button", { name: /4\. Permissions and safety/ }).click();
    check("permission prompts and scope safeguards are explained", (await page.getByRole("region", { name: "Permissions and safety" }).innerText()).toLowerCase().includes("network-scope safeguards"));
    await page.getByRole("button", { name: /5\. The catalog/ }).click();
    const catalog = page.getByRole("list", { name: "Catalog preview" });
    await catalog.waitFor();
    const n = await catalog.getByRole("listitem").count();
    check("curated catalog is shown from the bundled files", n === 6, `${n} tools`);
    await page.getByRole("button", { name: /6\. Your first tools/ }).click();
    const starters = page.getByRole("list", { name: "Starter tools" });
    await starters.waitFor();
    check("starter tools are offered", (await starters.getByRole("listitem").count()) >= 1, (await starters.innerText()).replace(/\s+/g, " ").slice(0, 200));
    await page.screenshot({ path: `${out}/04-starters.png` });
    await page.getByRole("button", { name: "Finish and go to the dashboard" }).click();
    await page.getByRole("heading", { level: 1, name: "Dashboard" }).waitFor();
    check("finishing goes to the dashboard", true);

    await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Settings", exact: true }).click();
    await page.getByText(/You are running/).waitFor();
    const ver = await page.getByText(/You are running/).innerText();
    check("Settings shows the application version", /0\.2\.0/.test(ver), ver);
    await page.getByRole("button", { name: "Show release notes" }).click();
    check("release notes are shown", (await page.getByLabel("Release notes").innerText()).includes("0.2.0"));
    const table = page.getByRole("table", { name: "Catalog entries" });
    check("catalog definitions list versions and digests", (await table.innerText()).includes("bundled with Vixalune") && /[0-9a-f]{12}/.test(await table.innerText()));
    await page.screenshot({ path: `${out}/05-settings.png`, fullPage: true });

    const nav = await page.evaluate(() => performance.getEntriesByType("navigation")[0]?.name);
    check("window loads the bundled frontend, not a dev server", /tauri\.localhost|^tauri:/.test(nav ?? page.url()), nav);
  } else if (scenario === "second-run") {
    await page.getByRole("heading", { level: 1, name: "Dashboard" }).waitFor({ timeout: 20000 });
    check("second launch goes straight to the dashboard (tour not repeated)", true);
    const info = await invoke("get_storage_info");
    const cfg = info.labs_dir;
    check("saved storage location survived the restart", /labs-chosen$/.test(cfg), cfg);
    const jobs = await invoke("list_jobs");
    check("job list readable", Array.isArray(jobs), `${jobs.length} job(s)`);
    const states = await invoke("list_tool_states");
    check("tool states readable", Array.isArray(states), JSON.stringify(states).slice(0, 200));
    const app = await invoke("get_app_info");
    check("startup notices reported by the backend", Array.isArray(app.startup_notices), JSON.stringify(app.startup_notices));
  } else if (scenario === "trivy-install") {
    await page.goto("http://tauri.localhost/#/catalog/trivy");
    await page.getByRole("heading", { level: 1, name: "Trivy" }).waitFor({ timeout: 20000 });
    const before = (await invoke("list_tool_states")).find((s) => s.tool_id === "trivy")?.state;
    check("Trivy starts not installed", before === "not_installed", String(before));
    await page.getByRole("button", { name: "Install", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Install Trivy?" });
    await dialog.waitFor();
    await dialog.getByText(/Method:/).waitFor({ timeout: 30000 });
    check("the install plan is shown before anything happens", /winget|package/i.test(await dialog.innerText()), (await dialog.innerText()).replace(/\s+/g, " ").slice(0, 220));
    await page.screenshot({ path: out + "/10-install-plan.png" });
    await dialog.getByRole("button", { name: "Install", exact: true }).click();
    // Wait for the job to finish (real winget download and install).
    let job;
    for (let i = 0; i < 240; i++) {
      const jobs = await invoke("list_jobs");
      job = jobs.find((j) => j.tool_id === "trivy" && j.kind === "install");
      if (job && ["succeeded", "failed", "cancelled"].includes(job.phase)) break;
      await new Promise((r) => setTimeout(r, 2000));
    }
    check("install job reached a final state", !!job && ["succeeded", "failed", "cancelled"].includes(job.phase), job && job.phase + ": " + job.message);
    check("install succeeded and was verified", job?.phase === "succeeded", job && job.message);
    const states = await invoke("list_tool_states");
    check("tool state is installed", states.find((s) => s.tool_id === "trivy")?.state === "installed", JSON.stringify(states.find((s) => s.tool_id === "trivy")));
    const installed = (await invoke("list_installed_tools")).find((t) => t.tool_id === "trivy");
    check("installed tool has a version and a location", !!installed?.version || !!installed?.location, JSON.stringify(installed));
    await page.screenshot({ path: out + "/11-installed.png" });
  } else if (scenario === "trivy-run") {
    await page.goto("http://tauri.localhost/#/run/trivy");
    const review = page.getByRole("button", { name: "Review what will run" });
    await review.waitFor({ timeout: 20000 });
    await page.getByLabel(/Folder to scan/).waitFor();
    // The sample project folder is created at start-up in the (changed) labs folder.
    const folder = await page.getByLabel(/Folder to scan/).inputValue();
    check("default folder is the practice sample project", /sample-code$/.test(folder), folder);
    await review.click();
    await page.getByLabel("Command preview").waitFor();
    check("preview shows the exact command", (await page.getByLabel("Command preview").innerText()).includes("trivy.exe fs"), (await page.getByLabel("Command preview").innerText()).slice(0, 160));
    await page.getByRole("checkbox", { name: /reviewed the command and target/ }).check();
    await page.getByRole("button", { name: "Run", exact: true }).click();
    const panel = page.getByLabel("Run output");
    await panel.waitFor();
    await panel.getByRole("button", { name: /Cancel run/ }).waitFor({ timeout: 30000 });
    check("a running tool shows live status and a Cancel button", true);
    await new Promise((r) => setTimeout(r, 4000));
    await page.screenshot({ path: out + "/12-running.png" });
    if (process.env.CANCEL_RUN) {
      await panel.getByRole("button", { name: /Cancel run/ }).click();
      await page.getByRole("dialog", { name: "Cancel this run?" }).getByRole("button", { name: "Cancel run" }).click();
      await panel.getByText("Cancelled", { exact: true }).first().waitFor({ timeout: 60000 });
      check("run was cancelled", true);
    } else {
      await panel.getByText(/Completed|Failed|Timed out|Cancelled/).first().waitFor({ timeout: 280000 });
      const text = (await panel.innerText()).replace(/\s+/g, " ");
      check("run finished", true, text.slice(0, 300));
      await page.screenshot({ path: out + "/13-finished.png", fullPage: true });
    }
    const runs = await invoke("list_runs", { limit: 5 });
    check("run is in the history with an outcome", !!runs[0]?.outcome, JSON.stringify({ outcome: runs[0]?.outcome, exit: runs[0]?.exit_code, scope: runs[0]?.scope, version: runs[0]?.tool_version }));
  } else if (scenario === "start-long-run") {
    // Leaves a run active so the caller can kill the application in the middle of it.
    await page.goto("http://tauri.localhost/#/run/trivy");
    const folderField = page.getByLabel(/Folder to scan/);
    await folderField.waitFor();
    await page.waitForFunction(() => document.querySelector("#param-folder")?.value);
    await folderField.fill(process.env.LONG_FOLDER);
    await page.getByRole("button", { name: "Review what will run" }).click();
    await page.getByRole("checkbox", { name: /reviewed the command and target/ }).check();
    await page.getByRole("button", { name: "Run", exact: true }).click();
    await page.getByLabel("Run output").getByRole("button", { name: /Cancel run/ }).waitFor({ timeout: 30000 });
    const jobs = await invoke("list_jobs");
    check("a run is active", jobs.some((j) => j.kind === "run" && j.phase === "running"), JSON.stringify(jobs.map((j) => [j.kind, j.phase])));
  } else if (scenario === "after-kill") {
    await page.getByRole("heading", { level: 1, name: "Dashboard" }).waitFor({ timeout: 30000 });
    const jobs = await invoke("list_jobs");
    const run = jobs.find((j) => j.kind === "run");
    check("the interrupted run is closed as failed (interrupted), not left running", run?.phase === "failed" && run?.error?.code === "interrupted", JSON.stringify(run && [run.phase, run.error?.code]));
    const runs = await invoke("list_runs", { limit: 5 });
    check("its history record is no longer shown as running", !!runs[0] && runs[0].outcome === "failed", JSON.stringify(runs[0] && [runs[0].outcome, runs[0].exit_code]));
    const states = await invoke("list_tool_states");
    check("the tool state is still truthful", states.find((s) => s.tool_id === "trivy")?.state === "installed", JSON.stringify(states.find((s) => s.tool_id === "trivy")));
    await page.goto("http://tauri.localhost/#/run/trivy");
    await page.getByRole("region", { name: "Recent runs" }).waitFor();
    await page.screenshot({ path: out + "/14-after-kill.png", fullPage: true });
    // The tool can be run again straight away (the stale job did not block it).
    await page.getByRole("button", { name: "Review what will run" }).click();
    await page.getByRole("checkbox", { name: /reviewed the command and target/ }).check();
    check("Run is enabled again after the interruption", await page.getByRole("button", { name: "Run", exact: true }).isEnabled());
  } else if (scenario === "trivy-uninstall") {
    await page.goto("http://tauri.localhost/#/catalog/trivy");
    await page.getByRole("heading", { level: 1, name: "Trivy" }).waitFor({ timeout: 20000 });
    await page.getByRole("button", { name: "Uninstall", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Uninstall Trivy?" });
    await dialog.waitFor();
    await dialog.getByRole("button", { name: "Uninstall", exact: true }).click();
    let job;
    for (let i = 0; i < 120; i++) {
      job = (await invoke("list_jobs")).find((j) => j.tool_id === "trivy" && j.kind === "uninstall");
      if (job && ["succeeded", "failed", "cancelled"].includes(job.phase)) break;
      await new Promise((r) => setTimeout(r, 2000));
    }
    check("uninstall succeeded", job?.phase === "succeeded", job && job.phase + ": " + job.message);
    const st = (await invoke("list_tool_states")).find((x) => x.tool_id === "trivy")?.state;
    check("tool state is not installed again", st === "not_installed", String(st));
  } else if (scenario === "version") {
    await page.getByRole("heading", { level: 1, name: "Dashboard" }).waitFor({ timeout: 30000 });
    const app = await invoke("get_app_info");
    check("version reported by the backend", app.version === process.env.EXPECT_VERSION, app.version);
    const jobs = await invoke("list_jobs");
    const runs = await invoke("list_runs", { limit: 20 });
    check("jobs and run history survived the update", jobs.length >= 1 && runs.length >= 1, jobs.length + " jobs, " + runs.length + " runs");
    const cfg = await invoke("get_storage_info");
    check("storage settings survived the update", /labs-chosen$/.test(cfg.labs_dir), cfg.labs_dir);
    check("startup notices", Array.isArray(app.startup_notices), JSON.stringify(app.startup_notices));
  } else {
    throw new Error("unknown scenario " + scenario);
  }
} catch (e) {
  check("scenario completed without error", false, String(e).slice(0, 400));
  await page.screenshot({ path: `${out}/error.png` }).catch(() => {});
}
writeFileSync(`${out}/${scenario}.json`, JSON.stringify(results, null, 2));
await browser.close();
process.exit(results.every((r) => r.ok) ? 0 : 1);
