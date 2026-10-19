//! Platform failures: turning raw tool output into an actionable diagnosis that says *which layer*
//! failed (Windows, WSL, the package manager inside it, or the container runtime) and what to do.
//!
//! Matching is on documented error codes and English message fragments. Localised Windows builds
//! print translated text, so unknown output degrades to a generic command failure that still
//! shows the raw output; it is never reported as success.

use serde::{Deserialize, Serialize};

pub const WSL_INSTALL_LINK: &str = "https://learn.microsoft.com/windows/wsl/install";
pub const WSL_TROUBLESHOOT_LINK: &str = "https://learn.microsoft.com/windows/wsl/troubleshooting";
pub const VIRTUALIZATION_LINK: &str = "https://support.microsoft.com/windows/enable-virtualization-on-windows-c5578302-6e43-4b4b-a449-8ced115f58e1";
pub const DOCKER_DESKTOP_LINK: &str = "https://docs.docker.com/desktop/setup/install/windows-install/";
pub const WINGET_LINK: &str = "https://learn.microsoft.com/windows/package-manager/winget/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    WslUnavailable,
    VirtualizationDisabled,
    DistributionMissing,
    ContainerRuntimeStopped,
    InsufficientDisk,
    PackageDownloadFailed,
    ProxyRestricted,
    UnsupportedArchitecture,
    MissingPermissions,
    PolicyRestricted,
    RemoteContainerContext,
    UnsupportedContainerOs,
    /// The distribution's package manager reported an error (unknown package, lock held, ...).
    PackageManagerError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureLayer {
    Windows,
    Wsl,
    PackageManager,
    ContainerRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformFailure {
    pub kind: FailureKind,
    pub layer: FailureLayer,
    /// Short evidence (the matching output line), never a full dump.
    pub detail: String,
}

impl PlatformFailure {
    pub fn new(kind: FailureKind, layer: FailureLayer, detail: impl Into<String>) -> Self {
        Self { kind, layer, detail: detail.into() }
    }

    pub fn code(&self) -> &'static str {
        match self.kind {
            FailureKind::WslUnavailable => "wsl_unavailable",
            FailureKind::VirtualizationDisabled => "virtualization_disabled",
            FailureKind::DistributionMissing => "distribution_missing",
            FailureKind::ContainerRuntimeStopped => "container_runtime_stopped",
            FailureKind::InsufficientDisk => "insufficient_disk_space",
            FailureKind::PackageDownloadFailed => "package_download_failed",
            FailureKind::ProxyRestricted => "proxy_restricted",
            FailureKind::UnsupportedArchitecture => "unsupported_architecture",
            FailureKind::MissingPermissions => "missing_permissions",
            FailureKind::PolicyRestricted => "policy_restricted",
            FailureKind::RemoteContainerContext => "remote_container_context",
            FailureKind::UnsupportedContainerOs => "unsupported_container_os",
            FailureKind::PackageManagerError => "package_manager_error",
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            FailureKind::WslUnavailable => "Windows Subsystem for Linux (WSL) is not available",
            FailureKind::VirtualizationDisabled => "Virtualization is turned off",
            FailureKind::DistributionMissing => "The Linux distribution was not found",
            FailureKind::ContainerRuntimeStopped => "The container engine is not running",
            FailureKind::InsufficientDisk => "There is not enough free disk space",
            FailureKind::PackageDownloadFailed => "A download failed",
            FailureKind::ProxyRestricted => "A network proxy is blocking the connection",
            FailureKind::UnsupportedArchitecture => "This tool does not support your processor architecture",
            FailureKind::MissingPermissions => "Permission was denied",
            FailureKind::PolicyRestricted => "A policy on this computer blocks this operation",
            FailureKind::RemoteContainerContext => "The container engine is not local",
            FailureKind::UnsupportedContainerOs => "Only Linux containers are supported",
            FailureKind::PackageManagerError => "The package manager reported an error",
        }
    }

    /// What the user should do. Always names a concrete next step.
    pub fn recovery(&self) -> &'static str {
        match self.kind {
            FailureKind::WslUnavailable => {
                "Open Setup and install WSL, then restart Windows if asked. Guide: https://learn.microsoft.com/windows/wsl/install"
            }
            FailureKind::VirtualizationDisabled => {
                "Turn on hardware virtualization (Intel VT-x / AMD-V / SVM) in your PC's BIOS or UEFI settings, then restart. This cannot be changed from Windows. Guide: https://support.microsoft.com/windows/enable-virtualization-on-windows-c5578302-6e43-4b4b-a449-8ced115f58e1"
            }
            FailureKind::DistributionMissing => {
                "Install a WSL 2 Linux distribution (Ubuntu is recommended) from Setup, or choose an existing one in Settings. Check names with 'wsl --list --verbose'."
            }
            FailureKind::ContainerRuntimeStopped => {
                "Start Docker Desktop and wait until it says it is running, then retry. If it will not start, check that WSL 2 works. Guide: https://docs.docker.com/desktop/troubleshoot-and-support/troubleshoot/"
            }
            FailureKind::InsufficientDisk => "Free up disk space (Windows Settings > System > Storage) and retry. Vixalune will not continue until enough is free.",
            FailureKind::PackageDownloadFailed => {
                "Check your internet connection, VPN and firewall, then retry. If you are behind a corporate proxy, ask your administrator for the proxy settings."
            }
            FailureKind::ProxyRestricted => {
                "Your network requires a proxy. Configure the proxy for Windows, WSL (http_proxy/https_proxy) and Docker Desktop (Settings > Resources > Proxies), or ask your IT administrator. Vixalune does not bypass proxies."
            }
            FailureKind::UnsupportedArchitecture => {
                "This tool or image has no build for your CPU architecture. Choose a tool that supports it; Vixalune will not emulate or fall back silently."
            }
            FailureKind::MissingPermissions => {
                "The operation needs permissions it was not given. If it needs administrator rights, approve the Windows prompt when it appears; if your organisation restricts this, contact your administrator."
            }
            FailureKind::PolicyRestricted => "Your organisation's policy blocks this. Vixalune does not bypass corporate policies; contact your IT administrator.",
            FailureKind::RemoteContainerContext => {
                "Vixalune only uses a container engine on this computer. Run 'docker context use default' (or desktop-linux) and unset DOCKER_HOST, then re-check."
            }
            FailureKind::UnsupportedContainerOs => "Switch Docker Desktop to Linux containers (tray icon > Switch to Linux containers), then re-check.",
            FailureKind::PackageManagerError => {
                "Read the package manager message in the job output. Common fixes: wait for another apt/dpkg process to finish, or check that the package name exists for your distribution."
            }
        }
    }
}

fn has(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Which tool produced the output, so package-manager and WSL problems are kept apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSource {
    Wsl,
    /// A Linux command (apt/dpkg) run inside WSL.
    LinuxPackageManager,
    Docker,
    Winget,
    /// A security tool launched by the user (nmap, trivy, ...).
    Tool,
}

/// Diagnose command output. `None` means "nothing recognised"; callers then show the raw output.
pub fn classify(source: OutputSource, exit_code: Option<i32>, lines: &[String]) -> Option<PlatformFailure> {
    use FailureKind as K;
    use FailureLayer as L;
    let text = lines.join("\n").to_lowercase();
    let evidence = |needles: &[&str]| -> String {
        lines.iter().find(|l| has(&l.to_lowercase(), needles)).cloned().unwrap_or_default()
    };
    let layer_default = match source {
        OutputSource::Wsl => L::Wsl,
        OutputSource::LinuxPackageManager => L::PackageManager,
        OutputSource::Docker => L::ContainerRuntime,
        OutputSource::Winget | OutputSource::Tool => L::Windows,
    };

    // Highest priority: problems that make everything else meaningless.
    let virt = ["0x80370102", "hcs_e_hyperv_not_installed", "virtualization is not enabled", "enable virtualization", "virtual machine platform", "virtualization must be enabled", "virtualization support is not enabled"];
    if has(&text, &virt) {
        return Some(PlatformFailure::new(K::VirtualizationDisabled, L::Windows, evidence(&virt)));
    }
    let wsl_missing = ["windows subsystem for linux is not installed", "wsl_e_wsl_optional_component_required", "wsl.exe --install", "wsl2 is not supported", "the windows subsystem for linux optional component is not enabled"];
    if matches!(source, OutputSource::Wsl | OutputSource::LinuxPackageManager) && has(&text, &wsl_missing) {
        return Some(PlatformFailure::new(K::WslUnavailable, L::Wsl, evidence(&wsl_missing)));
    }
    let distro = ["no distribution with the supplied name", "wsl_e_distro_not_found", "there is no distribution", "has no installed distributions"];
    if has(&text, &distro) {
        return Some(PlatformFailure::new(K::DistributionMissing, L::Wsl, evidence(&distro)));
    }
    let daemon = [
        "cannot connect to the docker daemon",
        "failed to connect to the docker api",
        "error during connect",
        "docker daemon is not running",
        "dockerdesktoplinuxengine: the system cannot find the file",
        "is the docker daemon running",
    ];
    if has(&text, &daemon) {
        return Some(PlatformFailure::new(K::ContainerRuntimeStopped, L::ContainerRuntime, evidence(&daemon)));
    }
    let disk = ["no space left on device", "not enough space on the disk", "disk full", "0x80070070", "insufficient disk space", "not enough free space"];
    if has(&text, &disk) {
        return Some(PlatformFailure::new(K::InsufficientDisk, layer_default, evidence(&disk)));
    }
    let proxy = ["407 proxy authentication", "proxy authentication required", "proxyconnect", "proxy error", "proxy blocked"];
    if has(&text, &proxy) {
        return Some(PlatformFailure::new(K::ProxyRestricted, layer_default, evidence(&proxy)));
    }
    let arch = ["no matching manifest for", "exec format error", "unsupported architecture", "does not support your architecture", "no installer for this system"];
    if has(&text, &arch) {
        return Some(PlatformFailure::new(K::UnsupportedArchitecture, layer_default, evidence(&arch)));
    }
    let perm = ["0x80070005", "access is denied", "permission denied", "requires elevation", "run as administrator", "operation not permitted", "requires root privileges", "requires administrator", "must be run as administrator", "got permission denied while trying to connect to the docker"];
    if has(&text, &perm) {
        return Some(PlatformFailure::new(K::MissingPermissions, layer_default, evidence(&perm)));
    }
    let net = [
        "0x80072ee7",
        "0x80072efd",
        "0x8007274c",
        "could not resolve",
        "temporary failure resolving",
        "temporary failure in name resolution",
        "network is unreachable",
        "failed to fetch",
        "connection timed out",
        "unable to connect",
        "tls handshake timeout",
        "i/o timeout",
        "dial tcp",
        "download failed",
    ];
    if has(&text, &net) {
        return Some(PlatformFailure::new(K::PackageDownloadFailed, layer_default, evidence(&net)));
    }
    if source == OutputSource::LinuxPackageManager {
        let apt = ["unable to locate package", "could not get lock", "dpkg was interrupted", "dpkg: error", "held broken packages"];
        let e_line = lines.iter().find(|l| l.starts_with("E: ")).cloned();
        if exit_code == Some(100) || e_line.is_some() || has(&text, &apt) {
            return Some(PlatformFailure::new(K::PackageManagerError, L::PackageManager, e_line.unwrap_or_else(|| evidence(&apt))));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn kind(source: OutputSource, code: i32, out: &[&str]) -> Option<(FailureKind, FailureLayer)> {
        classify(source, Some(code), &lines(out)).map(|f| (f.kind, f.layer))
    }

    #[test]
    fn wsl_not_installed() {
        let out = ["The Windows Subsystem for Linux is not installed. You can install by running 'wsl.exe --install'."];
        assert_eq!(kind(OutputSource::Wsl, 1, &out), Some((FailureKind::WslUnavailable, FailureLayer::Wsl)));
    }

    #[test]
    fn virtualization_disabled_wins_over_everything() {
        let out = ["WSL 2 requires virtualization", "Error code: Wsl/Service/CreateInstance/CreateVm/HCS/HCS_E_HYPERV_NOT_INSTALLED", "0x80370102"];
        assert_eq!(kind(OutputSource::Wsl, -1, &out), Some((FailureKind::VirtualizationDisabled, FailureLayer::Windows)));
        let f = classify(OutputSource::Wsl, Some(-1), &lines(&out)).unwrap();
        assert!(f.recovery().contains("BIOS") && f.recovery().contains("support.microsoft.com"));
    }

    #[test]
    fn distribution_missing() {
        let out = ["There is no distribution with the supplied name.", "Error code: Wsl/Service/WSL_E_DISTRO_NOT_FOUND"];
        assert_eq!(kind(OutputSource::Wsl, -1, &out), Some((FailureKind::DistributionMissing, FailureLayer::Wsl)));
    }

    #[test]
    fn docker_daemon_stopped_matches_the_real_docker_desktop_message() {
        let out = ["failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; check if the path is correct and if the daemon is running: open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified."];
        assert_eq!(kind(OutputSource::Docker, 1, &out), Some((FailureKind::ContainerRuntimeStopped, FailureLayer::ContainerRuntime)));
    }

    #[test]
    fn package_manager_errors_are_distinct_from_wsl_errors() {
        let apt = ["E: Unable to locate package nosuchtool"];
        assert_eq!(kind(OutputSource::LinuxPackageManager, 100, &apt), Some((FailureKind::PackageManagerError, FailureLayer::PackageManager)));
        let lock = ["E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 123"];
        assert_eq!(kind(OutputSource::LinuxPackageManager, 100, &lock).map(|k| k.0), Some(FailureKind::PackageManagerError));
        // Exit 100 alone (apt's generic failure) is still a package-manager problem.
        assert_eq!(kind(OutputSource::LinuxPackageManager, 100, &["something odd"]).map(|k| k.1), Some(FailureLayer::PackageManager));
        // The same exit code from wsl.exe itself is not misattributed.
        assert_eq!(kind(OutputSource::Wsl, 100, &["something odd"]), None);
    }

    #[test]
    fn network_proxy_disk_arch_and_permissions() {
        assert_eq!(kind(OutputSource::LinuxPackageManager, 100, &["E: Failed to fetch http://archive.ubuntu.com/x  Could not resolve 'archive.ubuntu.com'"]).map(|k| k.0), Some(FailureKind::PackageDownloadFailed));
        assert_eq!(kind(OutputSource::Docker, 1, &["Error response from daemon: Get \"https://registry-1.docker.io/v2/\": proxyconnect tcp: dial tcp 10.0.0.1:3128: connect: connection refused"]).map(|k| k.0), Some(FailureKind::ProxyRestricted));
        assert_eq!(kind(OutputSource::LinuxPackageManager, 100, &["E: Write error - write (28: No space left on device)"]).map(|k| k.0), Some(FailureKind::InsufficientDisk));
        assert_eq!(kind(OutputSource::Docker, 1, &["no matching manifest for linux/arm64/v8 in the manifest list entries"]).map(|k| k.0), Some(FailureKind::UnsupportedArchitecture));
        assert_eq!(kind(OutputSource::Winget, 1, &["Installer failed with exit code: 0x80070005 Access is denied"]).map(|k| k.0), Some(FailureKind::MissingPermissions));
        assert_eq!(kind(OutputSource::Winget, 1, &["0x8007274c A connection attempt failed"]).map(|k| k.0), Some(FailureKind::PackageDownloadFailed));
    }

    #[test]
    fn unrecognised_output_is_not_guessed() {
        assert_eq!(kind(OutputSource::Docker, 1, &["boom"]), None);
        assert_eq!(kind(OutputSource::Wsl, 0, &[]), None);
    }

    #[test]
    fn every_kind_has_code_title_and_a_concrete_recovery() {
        use FailureKind::*;
        for k in [
            WslUnavailable, VirtualizationDisabled, DistributionMissing, ContainerRuntimeStopped, InsufficientDisk, PackageDownloadFailed,
            ProxyRestricted, UnsupportedArchitecture, MissingPermissions, PolicyRestricted, RemoteContainerContext, UnsupportedContainerOs,
            PackageManagerError,
        ] {
            let f = PlatformFailure::new(k, FailureLayer::Wsl, "x");
            assert!(!f.code().is_empty() && !f.title().is_empty() && f.recovery().len() > 30, "{k:?}");
        }
    }
}


