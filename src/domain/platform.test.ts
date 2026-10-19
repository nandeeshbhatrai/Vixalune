import { createDemoBackend } from "@/infra/demoBackend";
import { allReady, formatBytes, isBlockedByDependency, toEnvironment } from "./platform";
import type { PlatformReport } from "./types";

async function report(): Promise<PlatformReport> {
  return createDemoBackend().getPlatformReport();
}

describe("toEnvironment", () => {
  it("derives prerequisite components from readiness, never from a bare CLI", async () => {
    const r = await report();
    const env = toEnvironment(r);
    const state = (c: string) => env.components.find((x) => x.component === c)?.state;
    expect(state("winget")).toBe("available");
    expect(state("wsl2")).toBe("missing");
    expect(state("docker")).toBe("missing"); // docker CLI is present but its engine is stopped
    expect(env.supported_os).toBe(true);
    expect(env.free_disk_bytes).toBe(r.disk.free_bytes);
  });

  it("keeps unknown as unknown and flags old Windows", async () => {
    const r = await report();
    r.readiness.containers.state = "unknown";
    r.package_managers = [];
    r.windows.build = 18363;
    const env = toEnvironment(r);
    expect(env.components.find((c) => c.component === "docker")?.state).toBe("unknown");
    expect(env.components.find((c) => c.component === "winget")?.state).toBe("unknown");
    expect(env.supported_os).toBe(false);
  });
});

describe("wizard helpers", () => {
  it("knows when an action waits for another", async () => {
    const r = await report();
    const distro = r.setup_actions.find((a) => a.id === "install_distro")!;
    const wsl = r.setup_actions.find((a) => a.id === "install_wsl")!;
    expect(isBlockedByDependency(distro, r.setup_actions)).toBe(true);
    expect(isBlockedByDependency(wsl, r.setup_actions)).toBe(false);
    expect(isBlockedByDependency(distro, [distro])).toBe(false); // WSL step already done
  });

  it("formats disk sizes", () => {
    expect(formatBytes(null)).toBe("unknown");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
    expect(formatBytes(600 * 1024 ** 3)).toBe("600 GB");
  });

  it("allReady only considers what the user chose", async () => {
    const r = await report();
    expect(allReady(r, { wsl: false, containers: false })).toBe(true); // Windows tools are ready
    expect(allReady(r, { wsl: true, containers: false })).toBe(false);
    expect(allReady(r, { wsl: false, containers: true })).toBe(false);
  });
});
