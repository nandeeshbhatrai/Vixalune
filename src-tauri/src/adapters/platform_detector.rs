//! Platform detection: Windows, virtualization, WSL, container engine, package managers, disk.
//!
//! Only documented, read-only commands are used (`wsl --status/--version/--list --verbose`,
//! `docker version/info/context inspect`, `winget --version`). Detection never starts a stopped
//! distribution, never elevates, and reports "unknown" rather than guessing.

use crate::domain::platform::*;
use crate::domain::process::{CancellationToken, OutputStream};
use crate::ports::{CommandExecutor, CommandOutcome, CommandSpec, EnvironmentDetector, ExecutableLocator, HostFacts, LineSink, PlatformDetector};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

struct Discard;
impl LineSink for Discard {
    fn line(&self, _: OutputStream, _: &str) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Ran { code: i32, lines: Vec<String> },
    NotFound,
    TimedOut,
    Failed(String),
}

impl Probe {
    fn ok(&self) -> bool {
        matches!(self, Probe::Ran { code: 0, .. })
    }
    fn lines(&self) -> &[String] {
        match self {
            Probe::Ran { lines, .. } => lines,
            _ => &[],
        }
    }
    fn first_line(&self) -> Option<String> {
        self.lines().iter().map(|l| l.trim_matches('\u{feff}').trim().to_string()).find(|l| !l.is_empty())
    }
}

// ------------------------------------------------------------------ parsers (pure, tested with real output)

fn clean(l: &str) -> String {
    l.trim_matches('\u{feff}').trim().to_string()
}

fn looks_like_version(t: &str) -> bool {
    let parts: Vec<&str> = t.split('.').collect();
    parts.len() >= 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// `WSL version: 2.3.26.0` -> `2.3.26.0` (label may be localised, so the number is located by shape).
pub fn parse_wsl_version(lines: &[String]) -> Option<String> {
    lines.iter().take(3).find_map(|l| l.split_whitespace().map(|t| t.trim_end_matches(',')).find(|t| looks_like_version(t)).map(String::from))
}

fn valid_distro_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Parse `wsl --list --verbose` rows from the right: `[*] NAME STATE VERSION`.
pub fn parse_wsl_list(lines: &[String]) -> Vec<WslDistro> {
    let mut out = Vec::new();
    for raw in lines {
        let l = clean(raw);
        let default = l.starts_with('*');
        let toks: Vec<&str> = l.trim_start_matches('*').split_whitespace().collect();
        if toks.len() < 3 {
            continue;
        }
        let Ok(version) = toks[toks.len() - 1].parse::<u8>() else { continue };
        if !(1..=2).contains(&version) {
            continue;
        }
        let name = toks[0];
        if !valid_distro_name(name) {
            continue;
        }
        let state = match toks[toks.len() - 2].to_ascii_lowercase().as_str() {
            "running" => DistroState::Running,
            "stopped" => DistroState::Stopped,
            "installing" => DistroState::Installing,
            _ => DistroState::Unknown,
        };
        out.push(WslDistro { name: name.to_string(), state, version: Some(version), is_default: default, internal: name.to_ascii_lowercase().starts_with("docker-desktop"), os_id: None });
    }
    out
}

pub fn parse_os_release_id(lines: &[String]) -> Option<String> {
    lines.iter().find_map(|l| l.strip_prefix("ID=").map(|v| v.trim().trim_matches('"').to_ascii_lowercase())).filter(|v| !v.is_empty() && v.len() <= 32)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerVersion {
    pub client: Option<String>,
    pub context: Option<String>,
    pub server: Option<(String, String, String)>, // version, os, arch
}

/// Output of `docker version --format "client={{.Client.Version}} context={{.Client.Context}} server=..."`.
pub fn parse_docker_version(lines: &[String]) -> Option<DockerVersion> {
    let line = lines.iter().map(|l| clean(l)).find(|l| l.starts_with("client="))?;
    let client = line.strip_prefix("client=")?.split_whitespace().next().map(String::from).filter(|v| v != "<no" && !v.is_empty());
    let context = line.split(" context=").nth(1).and_then(|r| r.split_whitespace().next()).map(String::from).filter(|c| !c.is_empty());
    let server = line.split(" server=").nth(1).map(str::trim).filter(|s| *s != "none").and_then(|s| {
        let p: Vec<&str> = s.split_whitespace().collect();
        (p.len() == 3).then(|| (p[0].to_string(), p[1].to_string(), p[2].to_string()))
    });
    Some(DockerVersion { client, context, server })
}

/// `docker info --format "{{.OSType}} {{.Architecture}} {{.SecurityOptions}}"`.
pub fn parse_docker_info(lines: &[String]) -> Option<(String, String, bool)> {
    let l = lines.iter().map(|l| clean(l)).find(|l| !l.is_empty() && !l.to_lowercase().starts_with("warning"))?;
    let mut it = l.splitn(3, ' ');
    let os = it.next()?.to_string();
    let arch = it.next()?.to_string();
    let sec = it.next().unwrap_or("");
    Some((os, arch, sec.contains("rootless")))
}

pub fn endpoint_is_local(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    h.starts_with("npipe:") || h.starts_with("unix:")
}

// ------------------------------------------------------------------ detector

pub struct SystemPlatformDetector {
    executor: Arc<dyn CommandExecutor>,
    locator: Arc<dyn ExecutableLocator>,
    facts: Arc<dyn HostFacts>,
    /// Folder whose volume is checked for free space.
    storage_dir: PathBuf,
    configured_distro: Option<String>,
}

impl SystemPlatformDetector {
    pub fn new(
        executor: Arc<dyn CommandExecutor>,
        locator: Arc<dyn ExecutableLocator>,
        facts: Arc<dyn HostFacts>,
        storage_dir: PathBuf,
        configured_distro: Option<String>,
    ) -> Self {
        Self { executor, locator, facts, storage_dir, configured_distro }
    }

    async fn probe(&self, program: Option<PathBuf>, args: &[&str]) -> Probe {
        let Some(program) = program else { return Probe::NotFound };
        let spec = CommandSpec { program, args: args.iter().map(|s| s.to_string()).collect(), timeout: PROBE_TIMEOUT };
        match self.executor.run(spec, &Discard, &CancellationToken::new()).await {
            Ok(r) => match r.outcome {
                CommandOutcome::Exited(code) => Probe::Ran { code, lines: r.tail },
                CommandOutcome::TimedOut => Probe::TimedOut,
                CommandOutcome::Cancelled => Probe::Failed("cancelled".into()),
            },
            Err(e) => Probe::Failed(e.to_string()),
        }
    }

    fn system32(&self, exe: &str) -> Option<PathBuf> {
        let root = std::env::var_os("SystemRoot")?;
        let p = PathBuf::from(root).join("System32").join(exe);
        p.exists().then_some(p).or_else(|| self.locator.locate(exe, &[]))
    }

    async fn detect_wsl(&self) -> WslInfo {
        let Some(wsl) = self.system32("wsl.exe") else {
            return WslInfo { installed: Some(false), detail: Some("wsl.exe was not found on this computer.".into()), wsl_version: None, default_version: None, distros: vec![] };
        };
        let (version, status) = tokio::join!(self.probe(Some(wsl.clone()), &["--version"]), self.probe(Some(wsl.clone()), &["--status"]));
        let installed = if version.ok() || status.ok() {
            Some(true)
        } else if matches!((&version, &status), (Probe::Ran { .. }, Probe::Ran { .. })) {
            Some(false)
        } else {
            None
        };
        match installed {
            Some(false) => {
                let detail = status.first_line().or_else(|| version.first_line());
                WslInfo { installed, detail, wsl_version: None, default_version: None, distros: vec![] }
            }
            None => WslInfo { installed, detail: Some("wsl.exe did not answer in time".into()), wsl_version: None, default_version: None, distros: vec![] },
            Some(true) => {
                let list = self.probe(Some(wsl.clone()), &["--list", "--verbose"]).await;
                let mut distros = parse_wsl_list(list.lines());
                // Identify the OS of distributions that are already running (never start one to find out).
                let ids = futures_join(distros.iter().filter(|d| d.state == DistroState::Running && !d.internal).map(|d| {
                    let name = d.name.clone();
                    let wsl = wsl.clone();
                    async move {
                        let p = self.probe(Some(wsl), &["--distribution", &name, "--exec", "/usr/bin/cat", "/etc/os-release"]).await;
                        (name, parse_os_release_id(p.lines()))
                    }
                }))
                .await;
                for (name, id) in ids {
                    if let Some(d) = distros.iter_mut().find(|d| d.name == name) {
                        d.os_id = id;
                    }
                }
                let default_version = status.lines().iter().find_map(|l| {
                    let t = clean(l);
                    let last = t.split(|c: char| c == ':' || c.is_whitespace()).filter(|s| !s.is_empty()).last()?.to_string();
                    (t.to_ascii_lowercase().contains("default") && t.to_ascii_lowercase().contains("version")).then(|| last.parse::<u8>().ok()).flatten()
                });
                WslInfo { installed, detail: None, wsl_version: parse_wsl_version(version.lines()), default_version, distros }
            }
        }
    }

    async fn detect_containers(&self) -> ContainerInfo {
        let Some(docker) = self.locator.locate("docker.exe", &[]) else { return ContainerInfo { runtime: None } };
        let fmt = "client={{.Client.Version}} context={{.Client.Context}} server={{if .Server}}{{.Server.Version}} {{.Server.Os}} {{.Server.Arch}}{{else}}none{{end}}";
        let ver_args = ["version", "--format", fmt];
        let ctx_args = ["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"];
        let (ver, ctx) = tokio::join!(self.probe(Some(docker.clone()), &ver_args), self.probe(Some(docker.clone()), &ctx_args));
        let parsed = parse_docker_version(ver.lines());
        let endpoint = ctx.first_line().filter(|_| ctx.ok());
        let env_host = self.facts.docker_host_env();
        let context_local = match (&env_host, &endpoint) {
            (Some(h), _) => Some(endpoint_is_local(h)),
            (None, Some(e)) => Some(endpoint_is_local(e)),
            (None, None) => None,
        };
        let daemon = match (&ver, parsed.as_ref().and_then(|p| p.server.clone())) {
            (Probe::TimedOut, _) => DaemonState::Unknown { reason: "docker did not answer in time".into() },
            (Probe::Failed(e), _) => DaemonState::Unknown { reason: e.clone() },
            (_, Some((server_version, os, arch))) => {
                let info = self.probe(Some(docker.clone()), &["info", "--format", "{{.OSType}} {{.Architecture}} {{.SecurityOptions}}"]).await;
                let (os_type, architecture, rootless) = match parse_docker_info(info.lines()) {
                    Some((o, a, r)) => (Some(o), Some(a), Some(r)),
                    None => (Some(os), Some(arch), None),
                };
                DaemonState::Running { server_version: Some(server_version), os_type, architecture, rootless }
            }
            (Probe::Ran { lines, .. }, None) => {
                let reason = lines.iter().map(|l| clean(l)).find(|l| !l.is_empty() && !l.starts_with("client=")).unwrap_or_else(|| "the engine did not respond".into());
                DaemonState::Stopped { reason }
            }
            (Probe::NotFound, None) => DaemonState::Unknown { reason: "docker not found".into() },
        };
        ContainerInfo {
            runtime: Some(RuntimeInfo {
                name: "docker".into(),
                cli_path: Some(docker.display().to_string()),
                cli_version: parsed.as_ref().and_then(|p| p.client.clone()),
                context: parsed.as_ref().and_then(|p| p.context.clone()),
                context_local,
                daemon,
            }),
        }
    }

    pub async fn collect(&self) -> PlatformReport {
        let (wsl, containers, winget) = tokio::join!(
            self.detect_wsl(),
            self.detect_containers(),
            self.probe(self.locator.locate("winget.exe", &[]), &["--version"])
        );
        let (build, display_version, edition) = self.facts.windows();
        let virt = Virtualization { firmware_enabled: self.facts.firmware_virtualization(), hypervisor_present: self.facts.hypervisor_present() };
        let disk = self.facts.disk(&self.storage_dir);
        let state = |b: Option<bool>| match b {
            Some(true) => FeatureState::Available,
            Some(false) => FeatureState::Unavailable,
            None => FeatureState::Unknown,
        };
        let features = vec![
            FeatureStatus { id: "virtualization_firmware".into(), label: "CPU virtualization (VT-x / AMD-V)".into(), state: state(virt.firmware_enabled), basis: "CPU feature flag reported by Windows".into() },
            FeatureStatus { id: "hypervisor".into(), label: "Windows hypervisor (used by WSL 2 and Docker Desktop)".into(), state: state(virt.hypervisor_present), basis: "CPUID hypervisor bit".into() },
            FeatureStatus { id: "wsl".into(), label: "Windows Subsystem for Linux".into(), state: state(wsl.installed), basis: "wsl.exe responds to --status/--version".into() },
            FeatureStatus {
                id: "optional_features".into(),
                label: "Virtual Machine Platform / WSL optional components".into(),
                state: FeatureState::Unknown,
                basis: "Reading Windows optional features needs administrator rights; Vixalune never elevates just to detect, so these are inferred from the rows above".into(),
            },
        ];
        // Missing (not found) is a fact; a timeout or failure leaves winget "unknown" (no entry).
        let mut package_managers = Vec::new();
        match &winget {
            Probe::Ran { code: 0, .. } => package_managers.push(PackageManagerInfo { name: "winget".into(), scope: "Windows".into(), version: winget.first_line() }),
            Probe::NotFound => package_managers.push(PackageManagerInfo { name: "winget".into(), scope: "Windows".into(), version: None }),
            _ => {}
        }
        if let Some(d) = wsl.usable_distros().find(|d| d.os_id.is_some()) {
            // Only recorded when we already know the distro family; apt itself is checked by the adapter.
            if matches!(d.os_id.as_deref(), Some("ubuntu" | "debian" | "kali")) {
                package_managers.push(PackageManagerInfo { name: "apt".into(), scope: format!("WSL: {}", d.name), version: None });
            }
        }
        PlatformReport::from_facts(Facts {
            detected_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            windows: WindowsInfo { build, display_version, edition, architecture: self.facts.architecture() },
            virtualization: virt,
            features,
            wsl,
            containers,
            package_managers,
            disk: DiskInfo { path: self.storage_dir.display().to_string(), free_bytes: disk.map(|d| d.0), total_bytes: disk.map(|d| d.1) },
            system: SystemNotes {
                pending_restart: self.facts.pending_restart(),
                proxy: self.facts.proxy(),
                policy_keys: self.facts.policy_keys(),
                is_elevated: self.facts.is_elevated(),
            },
            configured_distro: self.configured_distro.clone(),
        })
    }
}

/// Minimal join-all without an extra dependency.
async fn futures_join<F, T>(futs: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T>,
{
    let mut out = Vec::new();
    for f in futs {
        out.push(f.await);
    }
    out
}

#[async_trait]
impl PlatformDetector for SystemPlatformDetector {
    async fn detect(&self) -> PlatformReport {
        self.collect().await
    }
}

#[async_trait]
impl EnvironmentDetector for SystemPlatformDetector {
    async fn detect(&self) -> crate::domain::environment::EnvironmentReport {
        self.collect().await.to_environment()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::testing::{MockExecutor, MockLocator, Step};
    use crate::domain::environment::ComponentState;
    use crate::domain::manifest::Prerequisite;

    /// Real wsl.exe output on a PC without WSL (UTF-16 with NULs already stripped by the executor).
    const WSL_NOT_INSTALLED: &str = "The Windows Subsystem for Linux is not installed. You can install by running 'wsl.exe --install'.";
    const DOCKER_STOPPED_ERR: &str = "failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; check if the path is correct and if the daemon is running: open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified.";

    struct Facts(Option<bool>);
    impl HostFacts for Facts {
        fn windows(&self) -> (Option<u32>, Option<String>, Option<String>) {
            (Some(26300), Some("25H2".into()), Some("CoreSingleLanguage".into()))
        }
        fn architecture(&self) -> String {
            "x64".into()
        }
        fn firmware_virtualization(&self) -> Option<bool> {
            self.0
        }
        fn hypervisor_present(&self) -> Option<bool> {
            Some(true)
        }
        fn disk(&self, _: &std::path::Path) -> Option<(u64, u64)> {
            Some((600 << 30, 1000 << 30))
        }
        fn pending_restart(&self) -> Option<bool> {
            Some(false)
        }
        fn proxy(&self) -> Option<String> {
            None
        }
        fn policy_keys(&self) -> Vec<String> {
            vec![]
        }
        fn is_elevated(&self) -> Option<bool> {
            Some(false)
        }
        fn docker_host_env(&self) -> Option<String> {
            None
        }
    }

    fn detector(handler: impl Fn(&CommandSpec) -> Step + Send + Sync + 'static, docker: bool, virt: Option<bool>) -> SystemPlatformDetector {
        let mut loc = MockLocator::default();
        loc.add("winget.exe", "C:\\fake\\winget.exe");
        loc.add("wsl.exe", "C:\\fake\\wsl.exe");
        if docker {
            loc.add("docker.exe", "C:\\fake\\docker.exe");
        }
        SystemPlatformDetector::new(Arc::new(MockExecutor::new(handler)), Arc::new(loc), Arc::new(Facts(virt)), PathBuf::from("C:\\data"), None)
    }

    fn ctx_for(c: &CommandSpec) -> String {
        format!("{} {}", c.program.file_name().unwrap().to_string_lossy(), c.args.join(" "))
    }

    #[test]
    fn parses_wsl_list_rows_from_the_right_and_marks_defaults_and_internal_distros() {
        let lines: Vec<String> = [
            "  NAME              STATE           VERSION",
            "* Ubuntu-24.04      Running         2",
            "  Debian            Stopped         2",
            "  OldOne            Stopped         1",
            "  docker-desktop    Stopped         2",
            "\u{feff}garbage line without numbers",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let d = parse_wsl_list(&lines);
        assert_eq!(d.len(), 4);
        assert_eq!((d[0].name.as_str(), d[0].state, d[0].version, d[0].is_default), ("Ubuntu-24.04", DistroState::Running, Some(2), true));
        assert_eq!(d[1].state, DistroState::Stopped);
        assert_eq!(d[2].version, Some(1));
        assert!(d[3].internal && !d[0].internal);
    }

    #[test]
    fn parses_versions_and_docker_output() {
        assert_eq!(parse_wsl_version(&["WSL version: 2.3.26.0".into(), "Kernel version: 5.15.167.4-1".into()]).as_deref(), Some("2.3.26.0"));
        assert_eq!(parse_wsl_version(&["WSL-Version: 2.3.26.0".into()]).as_deref(), Some("2.3.26.0"));
        assert_eq!(parse_wsl_version(&[WSL_NOT_INSTALLED.into()]), None);
        assert_eq!(parse_os_release_id(&["NAME=\"Ubuntu\"".into(), "ID=ubuntu".into()]).as_deref(), Some("ubuntu"));
        let stopped = parse_docker_version(&["client=29.8.2 context=desktop-linux server=none".into(), DOCKER_STOPPED_ERR.into()]).unwrap();
        assert_eq!((stopped.client.as_deref(), stopped.context.as_deref(), stopped.server), (Some("29.8.2"), Some("desktop-linux"), None));
        let running = parse_docker_version(&["client=29.8.2 context=default server=29.8.2 linux amd64".into()]).unwrap();
        assert_eq!(running.server, Some(("29.8.2".into(), "linux".into(), "amd64".into())));
        assert_eq!(parse_docker_info(&["linux x86_64 [name=seccomp,profile=builtin name=cgroupns]".into()]), Some(("linux".into(), "x86_64".into(), false)));
        assert_eq!(parse_docker_info(&["linux x86_64 [name=rootless]".into()]).map(|r| r.2), Some(true));
        assert!(endpoint_is_local("npipe:////./pipe/docker_engine") && endpoint_is_local("unix:///var/run/docker.sock"));
        assert!(!endpoint_is_local("tcp://10.0.0.5:2375") && !endpoint_is_local("ssh://me@host"));
    }

    #[tokio::test]
    async fn reports_this_pc_wsl_absent_docker_installed_but_stopped() {
        let d = detector(
            |c| {
                let s = ctx_for(c);
                if s.starts_with("wsl.exe") {
                    Step::exit(1, &[WSL_NOT_INSTALLED, "", "For more information please visit https://aka.ms/wslinstall"])
                } else if s.starts_with("docker.exe version") {
                    Step::exit(1, &["client=29.8.2 context=desktop-linux server=none", DOCKER_STOPPED_ERR])
                } else if s.starts_with("docker.exe context") {
                    Step::ok(&["npipe:////./pipe/dockerDesktopLinuxEngine"])
                } else if s.starts_with("winget.exe") {
                    Step::ok(&["v1.9.25200"])
                } else {
                    panic!("unexpected probe {s}")
                }
            },
            true,
            Some(true),
        );
        let r = d.collect().await;
        assert_eq!(r.wsl.installed, Some(false));
        assert!(r.wsl.detail.as_deref().unwrap().contains("not installed"));
        let rt = r.containers.runtime.as_ref().unwrap();
        assert_eq!(rt.cli_version.as_deref(), Some("29.8.2"));
        assert_eq!(rt.context_local, Some(true));
        assert!(matches!(&rt.daemon, DaemonState::Stopped { reason } if reason.contains("dockerDesktopLinuxEngine")));
        assert_eq!(r.readiness.windows_tools.state, Readiness::Ready);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
        assert_eq!(r.readiness.containers.state, Readiness::NeedsSetup);
        assert_eq!(r.package_managers[0].version.as_deref(), Some("v1.9.25200"));
        assert_eq!(r.windows.build, Some(26300));
        assert_eq!(r.disk.free_bytes, Some(600 << 30));
        assert!(r.features.iter().any(|f| f.id == "optional_features" && f.state == FeatureState::Unknown));
        // The component view that gates installs agrees.
        let env = r.to_environment();
        assert_eq!(env.components.iter().find(|c| c.component == Prerequisite::Wsl2).unwrap().state, ComponentState::Missing);
    }

    #[tokio::test]
    async fn reports_a_healthy_machine_and_only_probes_running_distros() {
        let probed_stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = probed_stopped.clone();
        let d = detector(
            move |c| {
                let s = ctx_for(c);
                if s.contains("--distribution Debian") {
                    flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if s.contains("--exec /usr/bin/cat") {
                    Step::ok(&["NAME=\"Ubuntu\"", "ID=ubuntu"])
                } else if s.starts_with("wsl.exe --version") {
                    Step::ok(&["WSL version: 2.3.26.0"])
                } else if s.starts_with("wsl.exe --status") {
                    Step::ok(&["Default Distribution: Ubuntu-24.04", "Default Version: 2"])
                } else if s.starts_with("wsl.exe --list") {
                    Step::ok(&["  NAME            STATE    VERSION", "* Ubuntu-24.04    Running  2", "  Debian          Stopped  2"])
                } else if s.starts_with("docker.exe version") {
                    Step::ok(&["client=29.8.2 context=desktop-linux server=29.8.2 linux amd64"])
                } else if s.starts_with("docker.exe info") {
                    Step::ok(&["linux x86_64 [name=seccomp,profile=builtin]"])
                } else if s.starts_with("docker.exe context") {
                    Step::ok(&["npipe:////./pipe/dockerDesktopLinuxEngine"])
                } else {
                    Step::ok(&["v1.9.25200"])
                }
            },
            true,
            Some(true),
        );
        let r = d.collect().await;
        assert_eq!(r.wsl.installed, Some(true));
        assert_eq!(r.wsl.wsl_version.as_deref(), Some("2.3.26.0"));
        assert_eq!(r.wsl.default_version, Some(2));
        assert_eq!(r.wsl.distros.len(), 2);
        assert_eq!(r.wsl.distros[0].os_id.as_deref(), Some("ubuntu"));
        assert!(!probed_stopped.load(std::sync::atomic::Ordering::SeqCst), "a stopped distribution must never be started by detection");
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::Ready);
        assert_eq!(r.readiness.containers.state, Readiness::Ready);
        assert!(matches!(&r.containers.runtime.as_ref().unwrap().daemon, DaemonState::Running { rootless: Some(false), .. }));
        assert!(r.package_managers.iter().any(|p| p.name == "apt" && p.scope == "WSL: Ubuntu-24.04"));
    }

    #[tokio::test]
    async fn timeouts_and_missing_tools_are_unknown_or_absent_not_guessed() {
        let d = detector(|_| Step::timed_out(), true, None);
        let r = d.collect().await;
        assert_eq!(r.wsl.installed, None);
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::Unknown);
        assert!(matches!(r.containers.runtime.as_ref().unwrap().daemon, DaemonState::Unknown { .. }));
        assert_eq!(r.readiness.containers.state, Readiness::Unknown);
        assert!(r.package_managers.is_empty(), "a timed-out winget is unknown, not missing");
        assert_eq!(r.readiness.windows_tools.state, Readiness::Unknown);
        assert_eq!(r.virtualization.firmware_enabled, None);

        let d = detector(|_| Step::exit(1, &["x"]), false, Some(true));
        let r = d.collect().await;
        assert!(r.containers.runtime.is_none());
        assert_eq!(r.readiness.containers.state, Readiness::NeedsSetup);
    }

    #[tokio::test]
    async fn wsl_installed_without_distributions_and_remote_docker_context() {
        let d = detector(
            |c| {
                let s = ctx_for(c);
                if s.starts_with("wsl.exe --list") {
                    Step::exit(0, &["Windows Subsystem for Linux has no installed distributions."])
                } else if s.starts_with("wsl.exe") {
                    Step::ok(&["WSL version: 2.3.26.0"])
                } else if s.starts_with("docker.exe version") {
                    Step::ok(&["client=29.8.2 context=remote server=29.8.2 linux amd64"])
                } else if s.starts_with("docker.exe info") {
                    Step::ok(&["linux x86_64 []"])
                } else if s.starts_with("docker.exe context") {
                    Step::ok(&["ssh://me@build-box"])
                } else {
                    Step::ok(&["v1"])
                }
            },
            true,
            Some(true),
        );
        let r = d.collect().await;
        assert_eq!(r.wsl.installed, Some(true));
        assert!(r.wsl.distros.is_empty());
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
        assert_eq!(r.readiness.containers.state, Readiness::Blocked);
        assert!(r.readiness.containers.blockers[0].contains("remote"));
    }
}


