import { test } from "@playwright/test";

// Captures docs/screenshots/*.png from the labelled demo data. Opt in with: SCREENSHOTS=1 npm run test:e2e
test.skip(!process.env.SCREENSHOTS, "set SCREENSHOTS=1 to capture screenshots");
test.use({ viewport: { width: 1366, height: 768 }, colorScheme: "dark" });

const pages: [string, string][] = [
  ["dashboard", "/#/"],
  ["catalog", "/#/catalog"],
  ["tool-detail", "/#/catalog/semgrep"],
  ["installed", "/#/installed"],
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
