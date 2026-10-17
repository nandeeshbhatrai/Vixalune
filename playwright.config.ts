import { defineConfig } from "@playwright/test";

// E2E runs the Vite frontend against the in-browser mock backend (src/infra/mockApi.ts).
// It uses the system Microsoft Edge so no browser download is required.
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  reporter: "list",
  use: {
    baseURL: "http://localhost:1420",
    channel: "msedge",
  },
  webServer: {
    command: "npm run dev:demo",
    url: "http://localhost:1420",
    // Must not reuse a non-demo dev server: it would run against the unavailable backend.
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
