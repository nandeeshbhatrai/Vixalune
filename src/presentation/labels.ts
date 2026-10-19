import {
  Bug,
  FlaskConical,
  KeyRound,
  Network,
  Radar,
  Search,
  ShieldCheck,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import type { ToolStatus } from "@/domain/toolStatus";
import type {
  Category,
  InstallMethod,
  InstallSpec,
  ElevationNeed,
  IntegrityLevel,
  JobKind,
  JobPhase,
  VerificationLevel,
  VerificationOutcome,
  MaintenanceStatus,
  Prerequisite,
  Readiness,
  RuntimeKind,
  Locality,
  NetworkActivity,
  OutputFormat,
  RunOutcome,
} from "@/domain/types";

export const CATEGORY_LABEL: Record<Category, string> = {
  reconnaissance: "Reconnaissance",
  web_application: "Web application",
  network: "Network",
  passwords: "Passwords",
  vulnerability: "Vulnerability",
  forensics: "Forensics",
  utilities: "Utilities",
};

export const CATEGORY_ICON: Record<Category, LucideIcon> = {
  reconnaissance: Radar,
  web_application: Bug,
  network: Network,
  passwords: KeyRound,
  vulnerability: ShieldCheck,
  forensics: Search,
  utilities: Wrench,
};

export const RUNTIME_LABEL: Record<RuntimeKind, string> = {
  windows_native: "Windows",
  wsl: "WSL",
  container: "Container",
};

export const METHOD_LABEL: Record<InstallMethod, string> = {
  winget: "winget package",
  wsl_apt: "WSL apt package",
  container_image: "Container image",
  download_archive: "Verified download",
};

export const PREREQ_LABEL: Record<Prerequisite, string> = { winget: "winget", wsl2: "WSL 2", docker: "Docker" };

export const MAINTENANCE_LABEL: Record<MaintenanceStatus, string> = {
  active: "Actively maintained",
  stable: "Stable",
  deprecated: "Deprecated",
};

export type Tone = "neutral" | "info" | "success" | "warning" | "danger";

export const STATUS_META: Record<ToolStatus, { label: string; tone: Tone; description: string }> = {
  unknown: { label: "Status unknown", tone: "neutral", description: "Vixalune cannot yet detect whether this tool is installed." },
  not_installed: { label: "Not installed", tone: "neutral", description: "Not installed, and a prerequisite is missing or not yet checked." },
  ready_to_install: { label: "Ready to install", tone: "info", description: "All prerequisites are met." },
  installing: { label: "Installing", tone: "info", description: "An installation is in progress." },
  installed: { label: "Installed", tone: "success", description: "Installed and verified." },
  verification_failed: { label: "Verification failed", tone: "warning", description: "The tool is present but its check did not pass." },
  update_available: { label: "Update available", tone: "info", description: "A newer version exists. Updating is not available in this version yet." },
  uninstalling: { label: "Uninstalling", tone: "warning", description: "Removal is in progress." },
  error: { label: "Error", tone: "danger", description: "The last operation failed." },
};

export const JOB_KIND_LABEL: Record<JobKind, string> = { install: "Install", update: "Update", uninstall: "Uninstall", verify: "Verify", run: "Run", setup: "Setup" };

export const JOB_PHASE_META: Record<JobPhase, { label: string; tone: Tone }> = {
  queued: { label: "Queued", tone: "neutral" },
  checking_prerequisites: { label: "Checking prerequisites", tone: "info" },
  awaiting_consent: { label: "Awaiting approval", tone: "warning" },
  installing: { label: "Installing", tone: "info" },
  verifying: { label: "Verifying", tone: "info" },
  uninstalling: { label: "Uninstalling", tone: "warning" },
  running: { label: "Running", tone: "info" },
  cancelling: { label: "Cancelling", tone: "warning" },
  rolling_back: { label: "Rolling back", tone: "warning" },
  succeeded: { label: "Succeeded", tone: "success" },
  failed: { label: "Failed", tone: "danger" },
  cancelled: { label: "Cancelled", tone: "neutral" },
};

export const ACTIVE_PHASES: JobPhase[] = [
  "queued",
  "checking_prerequisites",
  "awaiting_consent",
  "installing",
  "verifying",
  "uninstalling",
  "running",
  "cancelling",
  "rolling_back",
];

export function isActivePhase(p: JobPhase): boolean {
  return ACTIVE_PHASES.includes(p);
}

export const PROVENANCE_LABEL = {
  official_upstream: "from the upstream project",
  package_repository: "via a package repository",
  community_maintained: "community maintained",
} as const;

export const READINESS_META: Record<Readiness, { label: string; tone: Tone }> = {
  ready: { label: "Ready", tone: "success" },
  needs_setup: { label: "Needs setup", tone: "warning" },
  blocked: { label: "Blocked", tone: "danger" },
  unknown: { label: "Unknown", tone: "neutral" },
};

export const ELEVATION_LABEL: Record<ElevationNeed, string> = {
  none: "Not needed",
  possible: "May be requested by the installer. You will be asked first.",
  required: "Required. Windows will ask you to approve.",
};

export const INTEGRITY_LABEL: Record<IntegrityLevel, string> = {
  package_manager: "Checked by the package manager (winget); Vixalune does not re-check it",
  sha256_pinned: "SHA-256 checked by Vixalune",
  image_digest: "Image digest checked by the container runtime",
  unavailable: "No integrity data is available",
};

export const VERIFICATION_OUTCOME_META: Record<VerificationOutcome, { label: string; tone: Tone }> = {
  passed: { label: "Passed", tone: "success" },
  failed: { label: "Failed", tone: "danger" },
  unavailable: { label: "Could not be checked", tone: "warning" },
};

export const VERIFICATION_LEVEL_LABEL: Record<VerificationLevel, string> = {
  executable: "Ran the installed program and checked its output",
  package_record: "Checked the package manager's record only",
  none: "No check completed",
};
/** Human description of an install method, for confirmation dialogs. Display only. */
export function describeInstall(spec: InstallSpec): string {
  switch (spec.method) {
    case "winget":
      return `winget package ${spec.package_id}`;
    case "wsl_apt":
      return `apt in WSL: ${spec.packages.join(", ")}`;
    case "container_image":
      return `container image ${spec.image} (pulled by digest)`;
    case "download_archive":
      return `checksum-verified download from ${new URL(spec.url).host}`;
  }
}

export const LAB_ICON = FlaskConical;

export function formatDuration(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return h > 0 ? `${h}h ${m}m` : m > 0 ? `${m}m ${sec}s` : `${sec}s`;
}

export function formatTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}




// ---------------------------------------------------------------- phase 5: runs and scope


/** A run that finished with findings is a success: the tool worked. Only "failed" and "timed out" are errors. */
export const RUN_OUTCOME_META: Record<RunOutcome, { label: string; tone: Tone; explanation: string }> = {
  clean: { label: "Completed, nothing reported", tone: "success", explanation: "The tool ran to the end and reported nothing of concern." },
  findings: { label: "Completed with findings", tone: "info", explanation: "The tool ran to the end and reported results (for example open ports or vulnerabilities). This is not an error." },
  failed: { label: "Failed", tone: "danger", explanation: "The tool could not run or ended with an exit code that this profile does not treat as success." },
  timed_out: { label: "Timed out", tone: "danger", explanation: "The run exceeded its time limit and was stopped." },
  cancelled: { label: "Cancelled", tone: "neutral", explanation: "You stopped the run." },
};

export const LOCALITY_LABEL: Record<Locality, string> = {
  loopback: "This computer",
  lab: "Your lab",
  private_network: "Private network (not on your lab list)",
  public: "Public internet",
  unspecified: "Not a specific target",
  special: "Reserved address",
  unresolvable: "Could not be resolved",
};

export const NETWORK_ACTIVITY_LABEL: Record<NetworkActivity, string> = {
  none: "No network traffic",
  probes_targets: "Sends traffic to the targets you named",
  downloads_data: "Downloads data (for example a vulnerability database)",
};

export const OUTPUT_FORMAT_LABEL: Record<OutputFormat, string> = { text: "Plain text", json: "JSON", xml: "XML", html: "HTML report", sarif: "SARIF" };
