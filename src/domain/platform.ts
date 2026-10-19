import type { ComponentStatus, EnvironmentReport, PlatformReport, Prerequisite, Readiness, SetupAction } from "./types";

/**
 * Component view used to decide whether a tool's prerequisites are met. Derived from the single
 * platform report so the two views can never disagree. Mirrors `PlatformReport::to_environment`.
 */
export function toEnvironment(r: PlatformReport): EnvironmentReport {
  const winget = r.package_managers.find((p) => p.name === "winget");
  const fromReadiness = (state: Readiness): { state: "available" | "missing" | "unknown"; reason?: string } =>
    state === "ready" ? { state: "available" } : state === "unknown" ? { state: "unknown", reason: "not determined" } : { state: "missing" };
  const comp = (component: Prerequisite, s: { state: "available" | "missing" | "unknown"; reason?: string }): ComponentStatus =>
    s.state === "available"
      ? { component, state: "available" }
      : s.state === "missing"
        ? { component, state: "missing" }
        : { component, state: "unknown", reason: s.reason ?? "not determined" };
  return {
    os: "windows",
    supported_os: r.windows.build == null || r.windows.build >= 19045,
    windows_build: r.windows.build,
    architecture: r.windows.architecture,
    free_disk_bytes: r.disk.free_bytes,
    components: [
      winget ? (winget.version ? { component: "winget", state: "available", version: winget.version } : { component: "winget", state: "missing" }) : { component: "winget", state: "unknown", reason: "not checked" },
      comp("wsl2", fromReadiness(r.readiness.linux_tools_wsl.state)),
      comp("docker", fromReadiness(r.readiness.containers.state)),
    ],
    issues: [],
  };
}

/** Setup steps not yet possible because another step must finish first. */
export function isBlockedByDependency(action: SetupAction, all: SetupAction[]): boolean {
  return action.depends_on.some((d) => all.some((a) => a.id === d));
}

export function formatBytes(n: number | null | undefined): string {
  if (n == null) return "unknown";
  const gb = n / 1024 ** 3;
  return gb >= 10 ? `${gb.toFixed(0)} GB` : `${gb.toFixed(1)} GB`;
}

/** Overall wizard outcome: everything the user chose is ready. */
export function allReady(r: PlatformReport, want: { wsl: boolean; containers: boolean }): boolean {
  return (
    r.readiness.windows_tools.state === "ready" &&
    (!want.wsl || r.readiness.linux_tools_wsl.state === "ready") &&
    (!want.containers || r.readiness.containers.state === "ready")
  );
}

