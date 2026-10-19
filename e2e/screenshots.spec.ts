import { test } from "@playwright/test";

// Captures docs/screenshots/*.png from the labelled demo data. Opt in with: SCREENSHOTS=1 npm run test:e2e
test.skip(!process.env.SCREENSHOTS, "set SCREENSHOTS=1 to capture screenshots");
test.use({ viewport: { width: 1366, height: 768 }, colorScheme: "dark" });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("vixalune.setupSeen", "1"));
});

const pages: [string, string][] = [
  ["setup", "/#/setup"],
  ["dashboard", "/#/"],
  ["catalog", "/#/catalog"],
  ["tool-detail", "/#/catalog/semgrep"],
  ["installed", "/#/installed"],
  ["run", "/#/run/nmap"],
  ["learn", "/#/learn"],
  ["tutorial", "/#/learn/nmap"],
  ["jobs", "/#/jobs"],
  ["labs", "/#/labs"],
  ["activity", "/#/activity"],
  ["settings", "/#/settings"],
];

for (const [name, path] of pages) {
  test(`screenshot ${name}`, async ({ page }) => {
    await page.goto(path);
    await page.getByRole("heading", { level: 1 }).first().waitFor();
    await page.waitForTimeout(600);
    await page.screenshot({ path: `docs/screenshots/${name}.png` });
  });
}

test("screenshot a finished run", async ({ page }) => {
  await page.goto("/#/run/nmap");
  await page.getByRole("button", { name: "Review what will run" }).click();
  await page.getByRole("checkbox", { name: /reviewed the command and target/ }).check();
  await page.getByRole("button", { name: "Run", exact: true }).click();
  await page.getByText("Completed with findings").first().waitFor({ timeout: 10_000 });
  await page.waitForTimeout(400);
  await page.screenshot({ path: "docs/screenshots/run-finished.png", fullPage: true });
});

test("screenshot light and dialog", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/#/catalog");
  await page.getByRole("heading", { level: 1 }).waitFor();
  await page.waitForTimeout(600);
  await page.screenshot({ path: "docs/screenshots/catalog-light.png" });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/#/catalog/trivy");
  await page.getByRole("button", { name: "Install" }).click();
  await page.getByRole("dialog").waitFor();
  await page.screenshot({ path: "docs/screenshots/install-confirm.png" });
});

