import { expect, test } from "@playwright/test";

test("dashboard renders and catalog is navigable", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible();
  await expect(page.getByText("WSL 2 was not found")).toBeVisible();

  await page.getByRole("link", { name: "Tool catalog" }).click();
  await page.getByLabel("Search tools").fill("nmap");
  await expect(page.getByText("Nmap")).toBeVisible();
  await page.getByText("Nmap").click();
  await expect(page.getByRole("heading", { name: "Nmap" })).toBeVisible();
});
