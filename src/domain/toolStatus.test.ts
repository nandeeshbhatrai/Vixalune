import { actionsFor, deriveStatus } from "./toolStatus";
import type { Capabilities, InstallState } from "./types";

const ALL_ON: Capabilities = { install: true, uninstall: true, verify: true, launch: true, jobs: true, labs: true, installed_detection: true, install_methods: ["winget"] };
const ALL_OFF: Capabilities = { install: false, uninstall: false, verify: false, launch: false, jobs: false, labs: false, installed_detection: false, install_methods: [] };

describe("deriveStatus", () => {
  it("never guesses: unknown backend state stays unknown", () => {
    expect(deriveStatus(null, [])).toBe("unknown");
    expect(deriveStatus(undefined, [])).toBe("unknown");
  });

  it("distinguishes not installed from ready to install using prerequisites", () => {
    expect(deriveStatus("not_installed", [])).toBe("ready_to_install");
    expect(deriveStatus("not_installed", ["docker"])).toBe("not_installed");
    expect(deriveStatus("not_installed", null)).toBe("not_installed"); // environment not checked
  });

  it.each<[InstallState, string]>([
    ["queued", "installing"],
    ["downloading", "installing"],
    ["installing", "installing"],
    ["verifying", "installing"],
    ["installed", "installed"],
    ["update_available", "update_available"],
    ["uninstalling", "uninstalling"],
    ["verification_failed", "verification_failed"],
    ["failed", "error"],
  ])("maps %s to %s", (state, expected) => {
    expect(deriveStatus(state, [])).toBe(expected);
  });
});

describe("actionsFor", () => {
  it("disables every action with a reason when the backend lacks the capability", () => {
    const actions = actionsFor("installed", ALL_OFF, []);
    expect(actions.map((a) => a.action)).toEqual(["launch", "verify", "uninstall"]);
    expect(actions.every((a) => !a.enabled && a.reason)).toBe(true);
  });

  it("enables actions when capabilities exist", () => {
    expect(actionsFor("installed", ALL_ON, []).every((a) => a.enabled)).toBe(true);
    expect(actionsFor("ready_to_install", ALL_ON, [])[0]).toEqual({ action: "install", enabled: true });
  });

  it("blocks install when prerequisites are missing and names them", () => {
    const [install] = actionsFor("not_installed", ALL_ON, ["docker"]);
    expect(install?.enabled).toBe(false);
    expect(install?.reason).toContain("docker");
  });

  it("offers no actions while an operation is running", () => {
    expect(actionsFor("installing", ALL_ON, [])).toEqual([]);
    expect(actionsFor("uninstalling", ALL_ON, [])).toEqual([]);
  });

  it("allows retry and uninstall after an error, verify and uninstall after failed verification", () => {
    expect(actionsFor("error", ALL_ON, []).map((a) => a.action)).toEqual(["install", "uninstall"]);
    expect(actionsFor("verification_failed", ALL_ON, []).map((a) => a.action)).toEqual(["verify", "uninstall"]);
  });
});

