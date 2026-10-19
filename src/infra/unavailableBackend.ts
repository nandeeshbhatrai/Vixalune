import type { Backend } from "@/domain/backend";
import type { AppErrorDto } from "@/domain/types";

const error: AppErrorDto = {
  code: "backend_unavailable",
  message: "The Vixalune backend is not running.",
  recovery:
    "Start the desktop app with 'npm run tauri:dev'. To explore the interface with clearly labelled demonstration data, run 'npm run dev:demo'.",
};

const fail = () => Promise.reject(error);

/** Used in a plain browser without the demo flag: every call fails visibly, nothing is invented. */
export const unavailableBackend: Backend = {
  isDemo: false,
  getAppInfo: fail,
  getCapabilities: fail,
  getPlatformReport: fail,
  listTools: fail,
  getTool: fail,
  listToolStates: fail,
  refreshToolStates: fail,
  listInstalledTools: fail,
  listJobs: fail,
  getJobOutput: fail,
  recentActivity: fail,
  listLabs: fail,
  startLab: fail,
  stopLab: fail,
  getLaunchDefaults: fail,
  getJob: fail,
  getRunOutput: fail,
  listRuns: fail,
  getRun: fail,
  saveRunOutput: fail,
  openRunFolder: fail,
  getSettings: fail,
  updateSettings: fail,
  purgeHistoryNow: fail,
  listTutorials: fail,
  getTutorial: fail,
  openTutorialLink: fail,
  getInstallPlan: fail,
  installTool: fail,
  uninstallTool: fail,
  verifyTool: fail,
  previewLaunch: fail,
  launchTool: fail,
  runSetupAction: fail,
  requestRestart: fail,
  cancelRestart: fail,
  openSetupLink: fail,
  approveJob: fail,
  cancelJob: fail,
  openLink: fail,
};


