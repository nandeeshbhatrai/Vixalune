import { expect, test } from "@playwright/test";

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
    ["Settings", "Settings"],
  ];
  for (const [link, heading] of screens) {
    await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: link }).click();
    await expect(page.getByRole("heading", { level: 1, name: heading })).toBeVisible();
  }
});

test("catalog search, filter and detail navigation", async ({ page }) => {
  await page.goto("/#/catalog");
  await expect(page.getByText("5 of 5 tools")).toBeVisible();
  await page.getByLabel("Search tools").fill("scanner");
  await expect(page.getByRole("heading", { name: "Nmap", level: 3 })).toBeVisible();
  await page.getByLabel("Search tools").fill("");
  await page.getByLabel("Category", { exact: true }).selectOption("vulnerability");
  await expect(page.getByText("2 of 5 tools")).toBeVisible();
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
