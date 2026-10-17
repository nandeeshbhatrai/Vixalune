import type { Capabilities, EnvironmentReport, InstallState, Prerequisite, ToolManifest } from "./types";

/** What the user sees about a tool. Derived from backend state plus environment readiness. */
export type ToolStatus =
  | "unknown"
  | "not_installed"
  | "ready_to_install"
  | "installing"
  | "installed"
  | "verification_failed"
  | "update_available"
  | "uninstalling"
  | "error";

export const ALL_STATUSES: ToolStatus[] = [
  "unknown",
  "not_installed",
  "ready_to_install",
  "installing",
  "installed",
  "verification_failed",
  "update_available",
  "uninstalling",
  "error",
];

/**
 * Prerequisites that the environment report says are missing for this tool.
 * Returns `null` when the environment has not been (or could not be) checked.
 */
export function missingPrerequisites(manifest: ToolManifest, env: EnvironmentReport | null | undefined): Prerequisite[] | null {
  if (!env) return null;
  return manifest.prerequisites.map((p) => p.kind).filter((p) => !env.components.some((c) => c.component === p && c.state === "available"));
}

/**
 * `state` null means the backend could not determine it: we show "unknown", never "not installed".
 * "ready_to_install" requires a *known* environment with all prerequisites met.
 */
export function deriveStatus(state: InstallState | null | undefined, missing: Prerequisite[] | null): ToolStatus {
  if (state == null) return "unknown";
  switch (state) {
    case "not_installed":
    case "cancelled":
      return missing !== null && missing.length === 0 ? "ready_to_install" : "not_installed";
    case "queued":
    case "downloading":
    case "installing":
    case "verifying":
    case "cancelling":
      return "installing";
    case "installed":
      return "installed";
    case "update_available":
      return "update_available";
    case "uninstalling":
      return "uninstalling";
    case "verification_failed":
      return "verification_failed";
    case "failed":
      return "error";
  }
}

export type ToolAction = "install" | "launch" | "verify" | "uninstall";

export interface ActionAvailability {
  action: ToolAction;
  enabled: boolean;
  /** Why the action is disabled, shown to the user. */
  reason?: string;
}

const NOT_AVAILABLE = "Not available in this version of Vixalune yet.";

/** Which actions apply to a status and whether the backend can perform them. */
export function actionsFor(
  status: ToolStatus,
  caps: Capabilities | null,
  missing: Prerequisite[] | null,
  installBlockedReason?: string,
): ActionAvailability[] {
  const cap = (enabled: boolean | undefined, action: ToolAction, extra?: string): ActionAvailability => {
    if (!caps || !enabled) return { action, enabled: false, reason: NOT_AVAILABLE };
    return extra ? { action, enabled: false, reason: extra } : { action, enabled: true };
  };
  switch (status) {
    case "unknown":
    case "not_installed":
    case "ready_to_install":
    case "error": {
      const blocked = installBlockedReason ?? (missing && missing.length > 0 ? `Missing prerequisites: ${missing.join(", ")}.` : undefined);
      return [cap(caps?.install, "install", blocked), ...(status === "error" ? [cap(caps?.uninstall, "uninstall")] : [])];
    }
    case "installed":
    case "update_available":
      return [cap(caps?.launch, "launch"), cap(caps?.verify, "verify"), cap(caps?.uninstall, "uninstall")];
    case "verification_failed":
      return [cap(caps?.verify, "verify"), cap(caps?.uninstall, "uninstall")];
    case "installing":
    case "uninstalling":
      return [];
  }
}

