//! Platform facts, readiness and setup actions.
//!
//! Three things are kept strictly apart:
//! 1. **Detection**: what was observed (`PlatformReport` facts). Unknown stays unknown.
//! 2. **Readiness**: what those facts mean for each capability (`CapabilityReadiness`).
//! 3. **Permission**: what Vixalune *may do about it*. A `SetupAction` is only a proposal; running
//!    one always requires explicit user confirmation, and none ever restarts Windows by itself.

use super::environment::{ComponentState, ComponentStatus, EnvironmentReport, Severity, SetupIssue};
use super::failure::{DOCKER_DESKTOP_LINK, VIRTUALIZATION_LINK, WINGET_LINK, WSL_INSTALL_LINK};
use super::manifest::Prerequisite;
use serde::{Deserialize, Serialize};

pub const MIN_WINDOWS_BUILD: u32 = 19045;
const GIB: u64 = 1024 * 1024 * 1024;
pub const MIN_DISK_WINDOWS: u64 = GIB;
pub const MIN_DISK_WSL: u64 = 8 * GIB;
pub const MIN_DISK_CONTAINERS: u64 = 5 * GIB;

// ------------------------------------------------------------------ facts

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsInfo {
    pub build: Option<u32>,
    pub display_version: Option<String>,
    pub edition: Option<String>,
    /// `x64` or `arm64`.
    pub architecture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Virtualization {
    /// Hardware virtualization enabled in firmware (`None` = could not be read).
    pub firmware_enabled: Option<bool>,
    /// A hypervisor is already running (true whenever WSL 2 / Hyper-V / Docker Desktop's VM can run).
    pub hypervisor_present: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureState {
    Available,
    Unavailable,
    Unknown,
}

/// A Windows capability and *how* its state was determined. Reading optional features directly
/// needs administrator rights, which detection never requests, so states are inferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureStatus {
    pub id: String,
    pub label: String,
    pub state: FeatureState,
    pub basis: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistroState {
    Running,
    Stopped,
    Installing,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslDistro {
    pub name: String,
    pub state: DistroState,
    pub version: Option<u8>,
    pub is_default: bool,
    /// Docker Desktop's own utility distributions; never offered for tools.
    pub internal: bool,
    /// `ID` from /etc/os-release. Read only for running distributions: detection never starts a stopped one.
    pub os_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslInfo {
    /// `None` = could not be determined.
    pub installed: Option<bool>,
    pub detail: Option<String>,
    pub wsl_version: Option<String>,
    pub default_version: Option<u8>,
    pub distros: Vec<WslDistro>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DaemonState {
    Running {
        server_version: Option<String>,
        os_type: Option<String>,
        architecture: Option<String>,
        rootless: Option<bool>,
    },
    Stopped { reason: String },
    Unknown { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub name: String,
    pub cli_path: Option<String>,
    pub cli_version: Option<String>,
    pub context: Option<String>,
    /// The selected engine endpoint is on this computer (named pipe / unix socket).
    pub context_local: Option<bool>,
    pub daemon: DaemonState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerInfo {
    /// `None` = no supported container CLI was found.
    pub runtime: Option<RuntimeInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManagerInfo {
    pub name: String,
    pub scope: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskInfo {
    pub path: String,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SystemNotes {
    pub pending_restart: Option<bool>,
    /// Proxy host:port if one is configured (credentials are never read or shown).
    pub proxy: Option<String>,
    /// Names of policy registry keys that may restrict WSL / containers.
    pub policy_keys: Vec<String>,
    /// Whether this process is elevated. Vixalune itself never runs elevated.
    pub is_elevated: Option<bool>,
}

// ------------------------------------------------------------------ readiness

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    Ready,
    /// Something is missing that the setup wizard can guide or perform.
    NeedsSetup,
    /// Cannot work until something outside Vixalune changes (firmware, policy, old Windows).
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReadiness {
    pub state: Readiness,
    pub summary: String,
    pub missing: Vec<String>,
    pub blockers: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessSummary {
    pub windows_tools: CapabilityReadiness,
    pub linux_tools_wsl: CapabilityReadiness,
    pub containers: CapabilityReadiness,
}

// ------------------------------------------------------------------ setup actions (proposals, not permission)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupActionId {
    InstallWsl,
    InstallDistro,
    ConvertDistroToWsl2,
    EnableVirtualization,
    InstallDockerDesktop,
    StartDockerDesktop,
    FreeDiskSpace,
    InstallWinget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupKind {
    /// Vixalune can run it after explicit confirmation.
    Automatic,
    /// The user does it; Vixalune only explains.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupAction {
    pub id: SetupActionId,
    pub title: String,
    pub why: String,
    pub kind: SetupKind,
    pub requires_admin: bool,
    pub requires_restart: bool,
    /// The exact command for automatic actions (shown before confirmation).
    pub command_preview: Option<String>,
    pub official_link: Option<String>,
    pub instructions: Vec<String>,
    /// Actions that must be finished first.
    pub depends_on: Vec<SetupActionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformReport {
    pub detected_at: String,
    pub windows: WindowsInfo,
    pub virtualization: Virtualization,
    pub features: Vec<FeatureStatus>,
    pub wsl: WslInfo,
    pub containers: ContainerInfo,
    pub package_managers: Vec<PackageManagerInfo>,
    pub disk: DiskInfo,
    pub system: SystemNotes,
    /// Distribution the user configured for tools, if any.
    pub configured_distro: Option<String>,
    pub readiness: ReadinessSummary,
    pub setup_actions: Vec<SetupAction>,
}

impl WslInfo {
    /// Distributions that can host tools: user-visible, running WSL 2.
    pub fn usable_distros(&self) -> impl Iterator<Item = &WslDistro> {
        self.distros.iter().filter(|d| !d.internal && d.version == Some(2))
    }
}

fn cap(state: Readiness, summary: &str) -> CapabilityReadiness {
    CapabilityReadiness { state, summary: summary.into(), missing: vec![], blockers: vec![], notes: vec![] }
}

fn gib(b: u64) -> String {
    format!("{:.1} GB", b as f64 / GIB as f64)
}

/// The fixed description of a setup step (including its official link). Links are never taken from the UI.
pub fn setup_action(id: SetupActionId) -> SetupAction {
    action(id)
}

fn action(id: SetupActionId) -> SetupAction {
    use SetupActionId as A;
    match id {
        A::InstallWsl => SetupAction {
            id,
            title: "Install Windows Subsystem for Linux (WSL 2)".into(),
            why: "Linux-based security tools run inside WSL 2, and Docker Desktop's Linux engine uses it on Windows Home.".into(),
            kind: SetupKind::Automatic,
            requires_admin: true,
            requires_restart: true,
            command_preview: Some("wsl.exe --install --no-distribution".into()),
            official_link: Some(WSL_INSTALL_LINK.into()),
            instructions: vec![
                "Windows will show an administrator (UAC) prompt for this one command.".into(),
                "Windows may ask you to restart afterwards. Vixalune never restarts without asking you first.".into(),
            ],
            depends_on: vec![],
        },
        A::InstallDistro => SetupAction {
            id,
            title: "Install a Linux distribution (Ubuntu 24.04)".into(),
            why: "WSL needs at least one Linux distribution to hold Linux tools.".into(),
            kind: SetupKind::Automatic,
            requires_admin: false,
            requires_restart: false,
            command_preview: Some("wsl.exe --install --distribution Ubuntu-24.04 --no-launch".into()),
            official_link: Some(WSL_INSTALL_LINK.into()),
            instructions: vec![
                "Downloads Ubuntu from Microsoft's distribution list (a few hundred MB).".into(),
                "You will not need to create a Linux user for Vixalune; tools are installed as root inside this distribution only.".into(),
            ],
            depends_on: vec![SetupActionId::InstallWsl],
        },
        A::ConvertDistroToWsl2 => SetupAction {
            id,
            title: "Convert your distribution to WSL 2".into(),
            why: "Tools are only installed into WSL 2 distributions.".into(),
            kind: SetupKind::Manual,
            requires_admin: false,
            requires_restart: false,
            command_preview: None,
            official_link: Some(WSL_INSTALL_LINK.into()),
            instructions: vec!["In a terminal run: wsl --set-version <distribution name> 2  (this can take several minutes).".into()],
            depends_on: vec![],
        },
        A::EnableVirtualization => SetupAction {
            id,
            title: "Turn on hardware virtualization in your BIOS/UEFI".into(),
            why: "WSL 2 and container engines run in a lightweight virtual machine, which needs CPU virtualization.".into(),
            kind: SetupKind::Manual,
            requires_admin: false,
            requires_restart: true,
            command_preview: None,
            official_link: Some(VIRTUALIZATION_LINK.into()),
            instructions: vec![
                "Restart and open your PC's firmware settings (often Esc, F2, F10 or Del at start-up).".into(),
                "Enable Intel Virtualization Technology (VT-x) or AMD SVM / AMD-V, save and exit.".into(),
                "On a work-managed PC this may be locked by your organisation; Vixalune cannot change it.".into(),
            ],
            depends_on: vec![],
        },
        A::InstallDockerDesktop => SetupAction {
            id,
            title: "Install a container engine (Docker Desktop)".into(),
            why: "Container-based tools and labs need a running Linux container engine.".into(),
            kind: SetupKind::Manual,
            requires_admin: true,
            requires_restart: false,
            command_preview: None,
            official_link: Some(DOCKER_DESKTOP_LINK.into()),
            instructions: vec![
                "Download the installer from docker.com and follow its steps.".into(),
                "Docker Desktop has its own licence terms (free for personal and small-business use; paid for larger companies). Read them before installing.".into(),
                "Vixalune does not install Docker for you.".into(),
            ],
            depends_on: vec![],
        },
        A::StartDockerDesktop => SetupAction {
            id,
            title: "Start Docker Desktop".into(),
            why: "Docker is installed but its engine is not running.".into(),
            kind: SetupKind::Manual,
            requires_admin: false,
            requires_restart: false,
            command_preview: None,
            official_link: Some("https://docs.docker.com/desktop/troubleshoot-and-support/troubleshoot/".into()),
            instructions: vec!["Open Docker Desktop from the Start menu and wait until it reports the engine is running, then re-check.".into()],
            depends_on: vec![],
        },
        A::FreeDiskSpace => SetupAction {
            id,
            title: "Free up disk space".into(),
            why: "There is not enough free space for the downloads this needs.".into(),
            kind: SetupKind::Manual,
            requires_admin: false,
            requires_restart: false,
            command_preview: None,
            official_link: Some("https://support.microsoft.com/windows/free-up-drive-space-in-windows-85529ccb-c365-490d-b548-831022bc9b32".into()),
            instructions: vec!["Open Windows Settings > System > Storage and remove files you no longer need.".into()],
            depends_on: vec![],
        },
        A::InstallWinget => SetupAction {
            id,
            title: "Install or update App Installer (winget)".into(),
            why: "Windows tools are installed with the Windows Package Manager.".into(),
            kind: SetupKind::Manual,
            requires_admin: false,
            requires_restart: false,
            command_preview: None,
            official_link: Some(WINGET_LINK.into()),
            instructions: vec!["Open the Microsoft Store, search for 'App Installer' and install or update it.".into()],
            depends_on: vec![],
        },
    }
}

impl PlatformReport {
    /// Fill `readiness` and `setup_actions` from the facts. Pure and deterministic.
    pub fn finalize(&mut self) {
        let (summary, mut actions) = derive(self);
        // De-duplicate and order so dependencies come first.
        let mut seen = std::collections::HashSet::new();
        actions.retain(|a| seen.insert(a.id));
        actions.sort_by_key(|a| (a.depends_on.len(), a.requires_restart as u8 ^ 1));
        self.readiness = summary;
        self.setup_actions = actions;
    }

    /// A report with only facts filled in; call [`finalize`](Self::finalize) afterwards.
    pub fn from_facts(f: Facts) -> Self {
        let mut r = Self {
            detected_at: f.detected_at,
            windows: f.windows,
            virtualization: f.virtualization,
            features: f.features,
            wsl: f.wsl,
            containers: f.containers,
            package_managers: f.package_managers,
            disk: f.disk,
            system: f.system,
            configured_distro: f.configured_distro,
            readiness: ReadinessSummary { windows_tools: cap(Readiness::Unknown, ""), linux_tools_wsl: cap(Readiness::Unknown, ""), containers: cap(Readiness::Unknown, "") },
            setup_actions: vec![],
        };
        r.finalize();
        r
    }

    pub fn has_winget(&self) -> Option<bool> {
        self.package_managers.iter().find(|p| p.name == "winget").map(|p| p.version.is_some())
    }

    /// The component view used by dependency resolution.
    pub fn to_environment(&self) -> EnvironmentReport {
        let winget = match self.package_managers.iter().find(|p| p.name == "winget") {
            Some(p) if p.version.is_some() => ComponentState::Available { version: p.version.clone() },
            Some(_) => ComponentState::Missing,
            None => ComponentState::Unknown { reason: "not checked".into() },
        };
        let from = |c: &CapabilityReadiness| match c.state {
            Readiness::Ready => ComponentState::Available { version: None },
            Readiness::NeedsSetup | Readiness::Blocked => ComponentState::Missing,
            Readiness::Unknown => ComponentState::Unknown { reason: c.summary.clone() },
        };
        let mut issues = Vec::new();
        for (name, c) in [("Windows tools", &self.readiness.windows_tools), ("Linux tools (WSL)", &self.readiness.linux_tools_wsl), ("Containers", &self.readiness.containers)] {
            let severity = match c.state {
                Readiness::Ready | Readiness::Unknown => continue,
                Readiness::NeedsSetup => Severity::Info,
                Readiness::Blocked => Severity::Blocking,
            };
            let first = c.blockers.first().or(c.missing.first()).cloned().unwrap_or_default();
            issues.push(SetupIssue {
                severity,
                title: format!("{name}: {}", c.summary),
                recovery: format!("{first}. Open Setup for guided steps."),
            });
        }
        EnvironmentReport {
            os: "windows".into(),
            supported_os: self.windows.build.is_none_or(|b| b >= MIN_WINDOWS_BUILD),
            windows_build: self.windows.build,
            architecture: self.windows.architecture.clone(),
            free_disk_bytes: self.disk.free_bytes,
            components: vec![
                ComponentStatus { component: Prerequisite::Winget, state: winget },
                ComponentStatus { component: Prerequisite::Wsl2, state: from(&self.readiness.linux_tools_wsl) },
                ComponentStatus { component: Prerequisite::Docker, state: from(&self.readiness.containers) },
            ],
            issues,
        }
    }
}

/// Observed facts, before derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub detected_at: String,
    pub windows: WindowsInfo,
    pub virtualization: Virtualization,
    pub features: Vec<FeatureStatus>,
    pub wsl: WslInfo,
    pub containers: ContainerInfo,
    pub package_managers: Vec<PackageManagerInfo>,
    pub disk: DiskInfo,
    pub system: SystemNotes,
    pub configured_distro: Option<String>,
}

fn disk_short(r: &PlatformReport, need: u64) -> Option<String> {
    r.disk.free_bytes.filter(|f| *f < need).map(|f| format!("only {} free on {} (about {} needed)", gib(f), r.disk.path, gib(need)))
}

fn policy_note(r: &PlatformReport) -> Option<String> {
    (!r.system.policy_keys.is_empty()).then(|| {
        format!(
            "Policy settings are present on this PC ({}). If setup fails, your organisation may be restricting it; Vixalune does not bypass policies.",
            r.system.policy_keys.join(", ")
        )
    })
}

fn derive(r: &PlatformReport) -> (ReadinessSummary, Vec<SetupAction>) {
    use SetupActionId as A;
    let mut actions: Vec<SetupAction> = Vec::new();

    // ---- Windows tools (winget)
    let mut win = cap(Readiness::Ready, "Ready to install Windows tools");
    if r.windows.build.is_some_and(|b| b < MIN_WINDOWS_BUILD) {
        win = cap(Readiness::Blocked, "This Windows version is too old");
        win.blockers.push(format!("Windows build {} is older than the minimum supported build {MIN_WINDOWS_BUILD} (Windows 10 22H2).", r.windows.build.unwrap_or(0)));
    } else if !matches!(r.windows.architecture.as_str(), "x64" | "arm64") {
        win = cap(Readiness::Blocked, "Unsupported processor architecture");
        win.blockers.push(format!("Architecture '{}' is not supported (x64 and arm64 only).", r.windows.architecture));
    } else {
        match r.has_winget() {
            Some(false) => {
                win = cap(Readiness::NeedsSetup, "winget is missing");
                win.missing.push("Windows Package Manager (winget)".into());
                actions.push(action(A::InstallWinget));
            }
            None => win = cap(Readiness::Unknown, "winget was not checked"),
            Some(true) => {}
        }
        if let Some(m) = disk_short(r, MIN_DISK_WINDOWS) {
            if win.state == Readiness::Ready {
                win = cap(Readiness::NeedsSetup, "Not enough disk space");
            }
            win.missing.push(m);
            actions.push(action(A::FreeDiskSpace));
        }
    }

    // ---- WSL
    let mut wsl = cap(Readiness::Ready, "A WSL 2 distribution is ready");
    wsl.notes.push("WSL shares your Windows files and network. It is not a security sandbox: only run tools you trust.".into());
    if r.virtualization.firmware_enabled == Some(false) && r.virtualization.hypervisor_present != Some(true) {
        wsl = cap(Readiness::Blocked, "Virtualization is turned off");
        wsl.blockers.push("Hardware virtualization is disabled in firmware; WSL 2 cannot start.".into());
        actions.push(action(A::EnableVirtualization));
    } else if r.windows.build.is_some_and(|b| b < MIN_WINDOWS_BUILD) {
        wsl = cap(Readiness::Blocked, "This Windows version is too old");
    } else {
        match r.wsl.installed {
            None => wsl = cap(Readiness::Unknown, "WSL could not be checked"),
            Some(false) => {
                wsl = cap(Readiness::NeedsSetup, "WSL is not installed");
                wsl.missing.push("Windows Subsystem for Linux".into());
                wsl.missing.push("A Linux distribution".into());
                actions.push(action(A::InstallWsl));
                actions.push(action(A::InstallDistro));
                if let Some(m) = disk_short(r, MIN_DISK_WSL) {
                    wsl.missing.push(m);
                    actions.push(action(A::FreeDiskSpace));
                }
            }
            Some(true) => {
                let user: Vec<&WslDistro> = r.wsl.distros.iter().filter(|d| !d.internal).collect();
                if user.is_empty() {
                    wsl = cap(Readiness::NeedsSetup, "No Linux distribution is installed");
                    wsl.missing.push("A Linux distribution".into());
                    actions.push(action(A::InstallDistro));
                    if let Some(m) = disk_short(r, MIN_DISK_WSL) {
                        wsl.missing.push(m);
                        actions.push(action(A::FreeDiskSpace));
                    }
                } else if r.wsl.usable_distros().next().is_none() {
                    wsl = cap(Readiness::NeedsSetup, "No distribution uses WSL 2");
                    wsl.missing.push(format!("A WSL 2 distribution (found: {})", user.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")));
                    actions.push(action(A::ConvertDistroToWsl2));
                    actions.push(action(A::InstallDistro));
                } else if let Some(want) = &r.configured_distro {
                    if !r.wsl.usable_distros().any(|d| &d.name == want) {
                        wsl = cap(Readiness::NeedsSetup, "The configured distribution was not found");
                        wsl.missing.push(format!("WSL 2 distribution '{want}' (set in Settings)"));
                    }
                }
            }
        }
        if wsl.state == Readiness::Ready {
            if let Some(d) = r.wsl.usable_distros().find(|d| d.state == DistroState::Stopped) {
                wsl.notes.push(format!("'{}' is stopped; it starts automatically when a tool is installed (detection never starts it).", d.name));
            }
            if r.system.pending_restart == Some(true) {
                wsl.notes.push("Windows has a restart pending; finish it before relying on WSL.".into());
            }
        }
    }
    if let Some(p) = policy_note(r) {
        wsl.notes.push(p);
    }

    // ---- Containers
    let mut con = cap(Readiness::Ready, "A local Linux container engine is running");
    match &r.containers.runtime {
        None => {
            con = cap(Readiness::NeedsSetup, "No container engine is installed");
            con.missing.push("A container engine (Docker)".into());
            actions.push(action(A::InstallDockerDesktop));
        }
        Some(rt) if rt.context_local == Some(false) => {
            con = cap(Readiness::Blocked, "The selected container engine is not on this computer");
            con.blockers.push("Docker's active context or DOCKER_HOST points to a remote engine. Vixalune only uses a local engine so labs cannot run on someone else's machine.".into());
        }
        Some(rt) => match &rt.daemon {
            DaemonState::Running { os_type, .. } if os_type.as_deref().is_some_and(|o| o != "linux") => {
                con = cap(Readiness::Blocked, "Windows containers are not supported");
                con.blockers.push("Docker is running in Windows-container mode. Switch to Linux containers.".into());
            }
            DaemonState::Running { rootless, .. } => {
                if *rootless != Some(true) {
                    con.notes.push("The engine runs as root inside its VM. Vixalune runs tool containers as a non-root user with dropped capabilities, but a container is not a safe sandbox for hostile code; use a separate VM for that.".into());
                }
            }
            DaemonState::Stopped { .. } => {
                if r.virtualization.firmware_enabled == Some(false) && r.virtualization.hypervisor_present != Some(true) {
                    con = cap(Readiness::Blocked, "Virtualization is turned off");
                    con.blockers.push("Hardware virtualization is disabled in firmware; the container engine cannot start.".into());
                    actions.push(action(A::EnableVirtualization));
                } else {
                    con = cap(Readiness::NeedsSetup, "The container engine is not running");
                    con.missing.push("A running container engine".into());
                    if r.wsl.installed == Some(false) {
                        con.notes.push("Docker Desktop needs WSL 2 (or Hyper-V on Pro/Enterprise editions) to start its Linux engine; WSL is not installed here.".into());
                        actions.push(action(A::InstallWsl));
                    }
                    actions.push(action(A::StartDockerDesktop));
                }
            }
            DaemonState::Unknown { reason } => con = cap(Readiness::Unknown, &format!("The container engine could not be checked: {reason}")),
        },
    }
    if con.state == Readiness::Ready {
        if let Some(m) = disk_short(r, MIN_DISK_CONTAINERS) {
            con = cap(Readiness::NeedsSetup, "Not enough disk space for images");
            con.missing.push(m);
            actions.push(action(A::FreeDiskSpace));
        }
    }
    if let Some(p) = policy_note(r) {
        con.notes.push(p);
    }

    (ReadinessSummary { windows_tools: win, linux_tools_wsl: wsl, containers: con }, actions)
}

#[cfg(test)]
pub mod fixtures {
    use super::*;

    /// A healthy machine: everything ready.
    pub fn ready_facts() -> Facts {
        Facts {
            detected_at: "2026-10-19T00:00:00Z".into(),
            windows: WindowsInfo { build: Some(26100), display_version: Some("24H2".into()), edition: Some("Pro".into()), architecture: "x64".into() },
            virtualization: Virtualization { firmware_enabled: Some(true), hypervisor_present: Some(true) },
            features: vec![],
            wsl: WslInfo {
                installed: Some(true),
                detail: None,
                wsl_version: Some("2.3.26".into()),
                default_version: Some(2),
                distros: vec![WslDistro { name: "Ubuntu-24.04".into(), state: DistroState::Running, version: Some(2), is_default: true, internal: false, os_id: Some("ubuntu".into()) }],
            },
            containers: ContainerInfo {
                runtime: Some(RuntimeInfo {
                    name: "docker".into(),
                    cli_path: Some("C:\\Docker\\docker.exe".into()),
                    cli_version: Some("29.8.2".into()),
                    context: Some("desktop-linux".into()),
                    context_local: Some(true),
                    daemon: DaemonState::Running { server_version: Some("29.8.2".into()), os_type: Some("linux".into()), architecture: Some("amd64".into()), rootless: Some(false) },
                }),
            },
            package_managers: vec![PackageManagerInfo { name: "winget".into(), scope: "Windows".into(), version: Some("v1.9".into()) }],
            disk: DiskInfo { path: "C:\\".into(), free_bytes: Some(200 * GIB), total_bytes: Some(500 * GIB) },
            system: SystemNotes::default(),
            configured_distro: None,
        }
    }

    /// This development PC as probed on 2026-10-19: WSL absent, Docker CLI present but stopped.
    pub fn this_pc_facts() -> Facts {
        let mut f = ready_facts();
        f.wsl = WslInfo { installed: Some(false), detail: Some("The Windows Subsystem for Linux is not installed.".into()), wsl_version: None, default_version: None, distros: vec![] };
        if let Some(rt) = f.containers.runtime.as_mut() {
            rt.daemon = DaemonState::Stopped { reason: "failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine".into() };
        }
        f.virtualization = Virtualization { firmware_enabled: Some(true), hypervisor_present: Some(true) };
        f
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn report(f: Facts) -> PlatformReport {
        PlatformReport::from_facts(f)
    }

    fn ids(r: &PlatformReport) -> Vec<SetupActionId> {
        r.setup_actions.iter().map(|a| a.id).collect()
    }

    #[test]
    fn healthy_machine_is_ready_everywhere_with_security_notes() {
        let r = report(ready_facts());
        assert_eq!(r.readiness.windows_tools.state, Readiness::Ready);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::Ready);
        assert_eq!(r.readiness.containers.state, Readiness::Ready);
        assert!(r.setup_actions.is_empty());
        assert!(r.readiness.linux_tools_wsl.notes.iter().any(|n| n.contains("not a security sandbox")));
        assert!(r.readiness.containers.notes.iter().any(|n| n.contains("not a safe sandbox")));
    }

    #[test]
    fn this_pc_needs_wsl_and_docker_cannot_start_without_it() {
        let r = report(this_pc_facts());
        assert_eq!(r.readiness.windows_tools.state, Readiness::Ready);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
        assert_eq!(r.readiness.containers.state, Readiness::NeedsSetup);
        let got = ids(&r);
        assert!(got.contains(&SetupActionId::InstallWsl) && got.contains(&SetupActionId::InstallDistro) && got.contains(&SetupActionId::StartDockerDesktop));
        assert!(r.readiness.containers.notes.iter().any(|n| n.contains("needs WSL 2")));
        // WSL comes before the distro that depends on it.
        let pos = |id| got.iter().position(|x| *x == id).unwrap();
        assert!(pos(SetupActionId::InstallWsl) < pos(SetupActionId::InstallDistro));
        // Environment view used by the resolver reflects it.
        let env = r.to_environment();
        let state = |p| env.components.iter().find(|c| c.component == p).unwrap().state.clone();
        assert_eq!(state(Prerequisite::Wsl2), ComponentState::Missing);
        assert_eq!(state(Prerequisite::Docker), ComponentState::Missing);
        assert!(matches!(state(Prerequisite::Winget), ComponentState::Available { .. }));
        assert!(env.issues.iter().all(|i| i.severity == Severity::Info));
    }

    #[test]
    fn disabled_virtualization_blocks_wsl_and_stopped_docker_but_not_a_running_one() {
        let mut f = this_pc_facts();
        f.virtualization = Virtualization { firmware_enabled: Some(false), hypervisor_present: Some(false) };
        let r = report(f);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::Blocked);
        assert_eq!(r.readiness.containers.state, Readiness::Blocked);
        assert!(ids(&r).contains(&SetupActionId::EnableVirtualization));
        assert!(!ids(&r).contains(&SetupActionId::InstallWsl), "do not propose installing what cannot run");
        assert!(r.to_environment().issues.iter().any(|i| i.severity == Severity::Blocking));

        // A hypervisor that is already running proves virtualization works despite a firmware read of false.
        let mut f = ready_facts();
        f.virtualization = Virtualization { firmware_enabled: Some(false), hypervisor_present: Some(true) };
        assert_eq!(report(f).readiness.linux_tools_wsl.state, Readiness::Ready);
    }

    #[test]
    fn wsl_without_distributions_or_with_only_wsl1() {
        let mut f = ready_facts();
        f.wsl.distros.clear();
        let r = report(f);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
        assert_eq!(ids(&r), vec![SetupActionId::InstallDistro]);

        let mut f = ready_facts();
        f.wsl.distros[0].version = Some(1);
        let r = report(f);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
        assert!(r.readiness.linux_tools_wsl.missing[0].contains("Ubuntu-24.04"));
        assert!(ids(&r).contains(&SetupActionId::ConvertDistroToWsl2));
    }

    #[test]
    fn docker_internal_distros_do_not_count_as_usable() {
        let mut f = ready_facts();
        f.wsl.distros = vec![WslDistro { name: "docker-desktop".into(), state: DistroState::Running, version: Some(2), is_default: false, internal: true, os_id: None }];
        assert_eq!(report(f).readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
    }

    #[test]
    fn configured_distribution_must_exist() {
        let mut f = ready_facts();
        f.configured_distro = Some("Debian".into());
        let r = report(f);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
        assert!(r.readiness.linux_tools_wsl.missing[0].contains("Debian"));
        let mut f = ready_facts();
        f.configured_distro = Some("Ubuntu-24.04".into());
        assert_eq!(report(f).readiness.linux_tools_wsl.state, Readiness::Ready);
    }

    #[test]
    fn stopped_distro_is_ready_and_explained() {
        let mut f = ready_facts();
        f.wsl.distros[0].state = DistroState::Stopped;
        let r = report(f);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::Ready);
        assert!(r.readiness.linux_tools_wsl.notes.iter().any(|n| n.contains("stopped")));
    }

    #[test]
    fn container_edge_cases() {
        let with = |f: &mut Facts, d: DaemonState, local: Option<bool>| {
            let rt = f.containers.runtime.as_mut().unwrap();
            rt.daemon = d;
            rt.context_local = local;
        };
        // No CLI.
        let mut f = ready_facts();
        f.containers.runtime = None;
        let r = report(f);
        assert_eq!(r.readiness.containers.state, Readiness::NeedsSetup);
        assert_eq!(r.setup_actions[0].kind, SetupKind::Manual, "Docker is never installed by Vixalune");
        // Remote context.
        let mut f = ready_facts();
        with(&mut f, DaemonState::Running { server_version: None, os_type: Some("linux".into()), architecture: None, rootless: None }, Some(false));
        let r = report(f);
        assert_eq!(r.readiness.containers.state, Readiness::Blocked);
        assert!(r.readiness.containers.blockers[0].contains("remote"));
        // Windows containers.
        let mut f = ready_facts();
        with(&mut f, DaemonState::Running { server_version: None, os_type: Some("windows".into()), architecture: None, rootless: None }, Some(true));
        assert_eq!(report(f).readiness.containers.state, Readiness::Blocked);
        // Unknown.
        let mut f = ready_facts();
        with(&mut f, DaemonState::Unknown { reason: "timed out".into() }, Some(true));
        assert_eq!(report(f).readiness.containers.state, Readiness::Unknown);
    }

    #[test]
    fn low_disk_needs_setup_and_names_the_numbers() {
        let mut f = this_pc_facts();
        f.disk.free_bytes = Some(2 * GIB);
        let r = report(f);
        assert!(r.readiness.linux_tools_wsl.missing.iter().any(|m| m.contains("2.0 GB free")));
        assert!(ids(&r).contains(&SetupActionId::FreeDiskSpace));
        let mut f = ready_facts();
        f.disk.free_bytes = Some(GIB / 2);
        let r = report(f);
        assert_eq!(r.readiness.windows_tools.state, Readiness::NeedsSetup);
        assert_eq!(r.readiness.containers.state, Readiness::NeedsSetup);
    }

    #[test]
    fn old_windows_missing_winget_and_unknowns() {
        let mut f = ready_facts();
        f.windows.build = Some(18363);
        let r = report(f);
        assert_eq!(r.readiness.windows_tools.state, Readiness::Blocked);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::Blocked);
        assert!(!r.to_environment().supported_os);

        let mut f = ready_facts();
        f.package_managers = vec![PackageManagerInfo { name: "winget".into(), scope: "Windows".into(), version: None }];
        assert_eq!(report(f).readiness.windows_tools.state, Readiness::NeedsSetup);

        let mut f = ready_facts();
        f.wsl.installed = None;
        f.package_managers.clear();
        let r = report(f);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::Unknown);
        assert_eq!(r.readiness.windows_tools.state, Readiness::Unknown);

        let mut f = ready_facts();
        f.windows.architecture = "x86".into();
        assert_eq!(report(f).readiness.windows_tools.state, Readiness::Blocked);
    }

    #[test]
    fn policies_and_pending_restart_are_reported_not_bypassed() {
        let mut f = ready_facts();
        f.system.policy_keys = vec!["HKLM\\SOFTWARE\\Policies\\Microsoft\\Windows\\WSL".into()];
        f.system.pending_restart = Some(true);
        let r = report(f);
        let notes = r.readiness.linux_tools_wsl.notes.join(" ");
        assert!(notes.contains("does not bypass policies") && notes.contains("restart pending"));
        assert!(r.readiness.containers.notes.iter().any(|n| n.contains("Policy")));
    }

    #[test]
    fn only_wsl_installation_is_automatic_and_it_is_the_only_admin_action_vixalune_runs() {
        let r = report(this_pc_facts());
        for a in &r.setup_actions {
            if a.kind == SetupKind::Automatic {
                assert!(a.command_preview.is_some(), "{:?} must show its exact command", a.id);
                assert!(matches!(a.id, SetupActionId::InstallWsl | SetupActionId::InstallDistro));
            } else {
                assert!(a.command_preview.is_none() && !a.instructions.is_empty());
            }
            assert!(a.official_link.as_deref().is_none_or(|l| l.starts_with("https://")));
        }
        assert!(r.setup_actions.iter().any(|a| a.id == SetupActionId::InstallWsl && a.requires_admin && a.requires_restart));
    }
}



