import { expect, test } from "@playwright/test";

// Pretend the first-run wizard was already seen, except in the test that covers it.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("vixalune.setupSeen", "1"));
});

// Runs against `npm run dev:demo`, so all data is demonstration data (and labelled as such).

test("every main screen renders", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Demonstration data")).toBeVisible();
  const screens: [string, string][] = [
    ["Dashboard", "Dashboard"],
    ["Tool Catalog", "Tool Catalog"],
    ["Installed Tools", "Installed Tools"],
    ["Running Jobs", "Running Jobs"],
    ["Learning Labs", "Learning Labs"],
    ["Activity", "Activity"],
    ["Setup", "Setup"],
    ["Settings", "Settings"],
  ];
  for (const [link, heading] of screens) {
    await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: link }).click();
    await expect(page.getByRole("heading", { level: 1, name: heading })).toBeVisible();
  }
});

test("catalog search, filter and detail navigation", async ({ page }) => {
  await page.goto("/#/catalog");
  await expect(page.getByText("6 of 6 tools")).toBeVisible();
  await page.getByLabel("Search tools").fill("scanner");
  await expect(page.getByRole("heading", { name: "Nmap", level: 3 })).toBeVisible();
  await page.getByLabel("Search tools").fill("");
  await page.getByLabel("Category", { exact: true }).selectOption("vulnerability");
  await expect(page.getByText("2 of 6 tools")).toBeVisible();
  await page.getByRole("link", { name: /Trivy/ }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Trivy" })).toBeVisible();
});

test("install asks for confirmation", async ({ page }) => {
  await page.goto("/#/catalog/trivy");
  await expect(page.getByRole("button", { name: "Install" })).toBeEnabled();
  await page.getByRole("button", { name: "Install" }).click();
  await expect(page.getByRole("dialog", { name: "Install Trivy?" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
});



test("first launch opens the setup wizard, explains and asks before changing anything", async ({ page }) => {
  await page.addInitScript(() => localStorage.removeItem("vixalune.setupSeen"));
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1, name: "Setup" })).toBeVisible();
  await expect(page.getByRole("list", { name: "Detected facts" })).toBeVisible();
  await page.getByRole("button", { name: /4\. Set up/ }).click();
  await page.getByRole("button", { name: /Review and run/ }).first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("wsl.exe --install --no-distribution")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Run this step" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
});

