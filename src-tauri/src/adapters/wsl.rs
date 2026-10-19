//! WSL 2 adapter for apt-based Linux tools (`InstallSpec::WslApt`).
//!
//! * The distribution is the configured one, else the default WSL 2 distribution. Docker's own
//!   utility distributions are never used. Nothing runs if no WSL 2 distribution exists.
//! * Every Linux command is `wsl.exe --distribution <name> --user root --cd /tmp --exec <argv…>`:
//!   an explicit working directory, no shell, validated package names, fixed absolute programs.
//!   `root` is root **inside that distribution only**; it grants nothing on Windows.
//! * Failures are diagnosed by layer: Windows/WSL (distribution missing, virtualization off, WSL
//!   not installed) versus the distribution's package manager (apt/dpkg), versus network/disk.
//! * WSL is **not** a sandbox: Linux tools see your Windows drives (`/mnt/c`) and network.
//! * Cleanup removes only the named packages (`apt-get remove`). Dependencies pulled in with them
//!   stay; `sudo apt-get autoremove` is left to the user so shared packages are never removed by surprise.

use crate::adapters::platform_detector::parse_wsl_list;
use crate::domain::failure::{classify, FailureKind, FailureLayer, OutputSource, PlatformFailure};
use crate::domain::jobs::{VerificationLevel, VerificationOutcome};
use crate::domain::manifest::{InstallMethod, InstallSpec, RuntimeKind, VerifyCheck};
use crate::domain::platform::{DistroState, WslDistro};
use crate::domain::process::OutputStream;
use crate::domain::{AppError, AppResult};
use crate::ports::{
    AdapterContext, CommandExecutor, CommandOutcome, CommandResult, CommandSpec, ExecutableLocator, InstallOutcome, InstallerAdapter, LineSink,
    PresenceStatus, PreflightReport, VerifyReport,
};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(120);
const APT_UPDATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const APT_INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const WORKDIR: &str = "/tmp";

pub struct WslAptAdapter {
    executor: Arc<dyn CommandExecutor>,
    locator: Arc<dyn ExecutableLocator>,
    /// Distribution chosen in Settings, if any.
    configured: Option<String>,
}

struct Discard;
impl LineSink for Discard {
    fn line(&self, _: OutputStream, _: &str) {}
}

fn packages(spec: &InstallSpec) -> AppResult<&[String]> {
    match spec {
        InstallSpec::WslApt { packages } => {
            let ok = !packages.is_empty()
                && packages.len() <= 20
                && packages.iter().all(|p| !p.is_empty() && p.len() <= 128 && !p.starts_with('-') && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '+')));
            if ok {
                Ok(packages)
            } else {
                Err(AppError::Validation("refusing unsafe apt package names".into()))
            }
        }
        _ => Err(AppError::Unsupported("the WSL adapter only handles apt packages".into())),
    }
}

fn valid_distro(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Fixed Linux argument vectors. Public for tests and the audit trail.
pub mod linux {
    pub fn has_apt() -> Vec<String> {
        vec!["/usr/bin/test".into(), "-x".into(), "/usr/bin/apt-get".into()]
    }
    pub fn dpkg_status(pkgs: &[String]) -> Vec<String> {
        let mut a = vec!["/usr/bin/dpkg-query".to_string(), "-W".into(), "-f=${db:Status-Abbrev}|${Package}|${Version}\\n".into(), "--".into()];
        a.extend(pkgs.iter().cloned());
        a
    }
    pub fn apt_update() -> Vec<String> {
        ["/usr/bin/env", "DEBIAN_FRONTEND=noninteractive", "/usr/bin/apt-get", "update", "-y"].map(String::from).to_vec()
    }
    pub fn apt_install(pkgs: &[String], upgrade: bool) -> Vec<String> {
        let mut a: Vec<String> = ["/usr/bin/env", "DEBIAN_FRONTEND=noninteractive", "/usr/bin/apt-get", "install", "-y", "--no-install-recommends"].map(String::from).to_vec();
        if upgrade {
            a.push("--only-upgrade".into());
        }
        a.push("--".into());
        a.extend(pkgs.iter().cloned());
        a
    }
    pub fn apt_remove(pkgs: &[String]) -> Vec<String> {
        let mut a: Vec<String> = ["/usr/bin/env", "DEBIAN_FRONTEND=noninteractive", "/usr/bin/apt-get", "remove", "-y", "--"].map(String::from).to_vec();
        a.extend(pkgs.iter().cloned());
        a
    }
}

/// `ii |nikto|1:2.1.5-3.1` rows -> (package, version) for installed packages only.
pub fn parse_dpkg_status(lines: &[String]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter_map(|l| {
            let mut it = l.trim_matches('\u{feff}').splitn(3, '|');
            let status = it.next()?.trim();
            let name = it.next()?.trim();
            let version = it.next()?.trim();
            (status == "ii" && !name.is_empty()).then(|| (name.to_string(), version.to_string()))
        })
        .collect()
}

impl WslAptAdapter {
    pub fn new(executor: Arc<dyn CommandExecutor>, locator: Arc<dyn ExecutableLocator>, configured: Option<String>) -> Self {
        Self { executor, locator, configured }
    }

    fn wsl_exe(&self) -> AppResult<PathBuf> {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let p = PathBuf::from(root).join("System32").join("wsl.exe");
            if p.exists() {
                return Ok(p);
            }
        }
        self.locator.locate("wsl.exe", &[]).ok_or_else(|| {
            AppError::Platform(PlatformFailure::new(FailureKind::WslUnavailable, FailureLayer::Wsl, "wsl.exe was not found"))
        })
    }

    async fn run_wsl(&self, ctx: &AdapterContext<'_>, args: Vec<String>, timeout: Duration, quiet: bool) -> AppResult<CommandResult> {
        if ctx.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let spec = CommandSpec { program: self.wsl_exe()?, args, timeout };
        if !quiet {
            ctx.sink.line(OutputStream::Stdout, &format!("> wsl.exe {}", spec.args.join(" ")));
        }
        let discard = Discard;
        let sink: &dyn LineSink = if quiet { &discard } else { ctx.sink };
        self.executor.run(spec, sink, ctx.cancel).await
    }

    /// Choose the distribution. Errors are WSL-layer diagnoses (not installed, no distribution, WSL 1 only).
    async fn resolve_distro(&self, ctx: &AdapterContext<'_>) -> AppResult<WslDistro> {
        let r = self.run_wsl(ctx, vec!["--list".into(), "--verbose".into()], READ_TIMEOUT, true).await?;
        let lines = r.tail.clone();
        match r.outcome {
            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
            CommandOutcome::TimedOut => return Err(AppError::TimedOut("wsl.exe did not answer".into())),
            CommandOutcome::Exited(code) if code != 0 && parse_wsl_list(&lines).is_empty() => {
                let f = classify(OutputSource::Wsl, Some(code), &lines)
                    .unwrap_or_else(|| PlatformFailure::new(FailureKind::WslUnavailable, FailureLayer::Wsl, lines.first().cloned().unwrap_or_default()));
                return Err(AppError::Platform(f));
            }
            _ => {}
        }
        let distros = parse_wsl_list(&lines);
        let usable: Vec<&WslDistro> = distros.iter().filter(|d| !d.internal && d.version == Some(2)).collect();
        let chosen = match &self.configured {
            Some(want) => usable.iter().find(|d| &d.name == want).copied().ok_or_else(|| {
                AppError::Platform(PlatformFailure::new(
                    FailureKind::DistributionMissing,
                    FailureLayer::Wsl,
                    format!("the configured distribution '{want}' is not an installed WSL 2 distribution"),
                ))
            })?,
            None => usable.iter().find(|d| d.is_default).or(usable.first()).copied().ok_or_else(|| {
                let detail = if distros.iter().any(|d| !d.internal) { "installed distributions use WSL 1; tools need WSL 2" } else { "no Linux distribution is installed" };
                AppError::Platform(PlatformFailure::new(FailureKind::DistributionMissing, FailureLayer::Wsl, detail))
            })?,
        };
        if !valid_distro(&chosen.name) {
            return Err(AppError::Validation(format!("refusing unsafe distribution name '{}'", chosen.name)));
        }
        Ok(chosen.clone())
    }

    /// Run a Linux argv inside the distribution as root, in `/tmp`, without a shell.
    async fn linux(&self, ctx: &AdapterContext<'_>, distro: &WslDistro, argv: Vec<String>, timeout: Duration, quiet: bool) -> AppResult<CommandResult> {
        let mut args: Vec<String> = ["--distribution", distro.name.as_str(), "--user", "root", "--cd", WORKDIR, "--exec"].map(String::from).to_vec();
        args.extend(argv);
        self.run_wsl(ctx, args, timeout, quiet).await
    }

    /// Turn a finished command into Ok(result) or a diagnosed error that names the failing layer.
    fn expect(&self, r: CommandResult, what: &str) -> AppResult<CommandResult> {
        match r.outcome {
            CommandOutcome::Exited(0) => Ok(r),
            CommandOutcome::Cancelled => Err(AppError::Cancelled),
            CommandOutcome::TimedOut => Err(AppError::TimedOut(format!("{what} did not finish in time"))),
            CommandOutcome::Exited(code) => {
                if let Some(f) = classify(OutputSource::LinuxPackageManager, Some(code), &r.tail) {
                    return Err(AppError::Platform(f));
                }
                let start = r.tail.len().saturating_sub(6);
                Err(AppError::CommandFailed { code: Some(code), message: format!("{what} exited with code {code}. {}", r.tail[start..].join(" | ")) })
            }
        }
    }

    async fn status_in(&self, ctx: &AdapterContext<'_>, distro: &WslDistro, pkgs: &[String]) -> AppResult<PresenceStatus> {
        let r = self.linux(ctx, distro, linux::dpkg_status(pkgs), READ_TIMEOUT, true).await?;
        match r.outcome {
            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
            CommandOutcome::TimedOut => return Ok(PresenceStatus::Unknown("dpkg-query timed out".into())),
            CommandOutcome::Exited(_) => {}
        }
        let installed = parse_dpkg_status(&r.tail);
        if installed.len() == pkgs.len() && pkgs.iter().all(|p| installed.iter().any(|(n, _)| n == p)) {
            return Ok(PresenceStatus::Installed { version: installed.first().map(|(_, v)| v.clone()) });
        }
        // Not installed (dpkg answers with "no packages found" and exit code 1). A WSL-level
        // problem looks different, so diagnose before concluding.
        if let CommandOutcome::Exited(code) = r.outcome {
            if let Some(f) = classify(OutputSource::Wsl, Some(code), &r.tail) {
                return Ok(PresenceStatus::Unknown(format!("{}: {}", f.title(), f.detail)));
            }
        }
        Ok(PresenceStatus::NotInstalled)
    }
}

#[async_trait]
impl InstallerAdapter for WslAptAdapter {
    fn method(&self) -> InstallMethod {
        InstallMethod::WslApt
    }

    fn supports(&self, spec: &InstallSpec) -> bool {
        matches!(spec, InstallSpec::WslApt { .. })
    }

    async fn preflight(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PreflightReport> {
        let pkgs = packages(spec)?;
        let distro = self.resolve_distro(ctx).await?;
        let mut notes = vec![format!("Will install into WSL distribution '{}'. WSL shares your Windows files and network and is not a security sandbox.", distro.name)];
        if distro.state == DistroState::Stopped {
            notes.push(format!("'{}' is stopped and will be started.", distro.name));
        }
        // apt must exist: this also proves the distribution can start and run commands.
        let r = self.linux(ctx, &distro, linux::has_apt(), READ_TIMEOUT, true).await?;
        match r.outcome {
            CommandOutcome::Exited(0) => {}
            CommandOutcome::Exited(code) => {
                if let Some(f) = classify(OutputSource::Wsl, Some(code), &r.tail) {
                    return Err(AppError::Platform(f));
                }
                return Err(AppError::Unsupported(format!(
                    "the distribution '{}' has no apt-get; only Debian/Ubuntu-family distributions are supported for this tool",
                    distro.name
                )));
            }
            CommandOutcome::TimedOut => return Err(AppError::TimedOut(format!("'{}' did not start in time", distro.name))),
            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
        }
        let (installed, version) = match self.status_in(ctx, &distro, pkgs).await? {
            PresenceStatus::Installed { version } => (true, version),
            _ => (false, None),
        };
        Ok(PreflightReport { already_installed: installed, installed_version: version, available_version: None, notes })
    }

    async fn install(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec, update: bool) -> AppResult<InstallOutcome> {
        let pkgs = packages(spec)?;
        let distro = self.resolve_distro(ctx).await?;
        // Refresh package lists first, otherwise a fresh distribution cannot find anything.
        let r = self.linux(ctx, &distro, linux::apt_update(), APT_UPDATE_TIMEOUT, false).await?;
        self.expect(r, "apt-get update")?;
        let r = self.linux(ctx, &distro, linux::apt_install(pkgs, update), APT_INSTALL_TIMEOUT, false).await?;
        self.expect(r, "apt-get install")?;
        Ok(InstallOutcome { detail: format!("installed {} in {}", pkgs.join(", "), distro.name), exit_code: Some(0) })
    }

    async fn status(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PresenceStatus> {
        let pkgs = packages(spec)?;
        let distro = match self.resolve_distro(ctx).await {
            Ok(d) => d,
            Err(AppError::Platform(f)) => return Ok(PresenceStatus::Unknown(format!("{}: {}", f.title(), f.detail))),
            Err(e) => return Err(e),
        };
        self.status_in(ctx, &distro, pkgs).await
    }

    async fn verify(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<VerifyReport> {
        use VerificationLevel as L;
        use VerificationOutcome as O;
        let pkgs = packages(spec)?;
        let distro = self.resolve_distro(ctx).await?;
        let location = Some(format!("WSL: {}", distro.name));
        let mut outcome = O::Passed;
        let mut level = L::None;
        let mut details = Vec::new();
        for check in &ctx.tool.verify.checks {
            let (o, l, d) = match check {
                VerifyCheck::PackageInstalled => match self.status_in(ctx, &distro, pkgs).await? {
                    PresenceStatus::Installed { version } => (O::Passed, L::PackageRecord, format!("dpkg lists {} as installed{}", pkgs.join(", "), version.map(|v| format!(" (version {v})")).unwrap_or_default())),
                    PresenceStatus::NotInstalled => (O::Failed, L::PackageRecord, "dpkg does not list the package as installed".into()),
                    PresenceStatus::Unknown(why) => (O::Unavailable, L::None, format!("package record could not be checked: {why}")),
                },
                VerifyCheck::ExecutableOutput { runtime, executable, args, expect_output_contains, .. } => {
                    if *runtime != RuntimeKind::Wsl {
                        (O::Unavailable, L::None, format!("check for runtime {runtime:?} is not handled by the WSL adapter"))
                    } else {
                        let mut argv = vec![executable.clone()];
                        argv.extend(args.iter().cloned());
                        let timeout = Duration::from_secs(u64::from(ctx.tool.verify.timeout_secs.unwrap_or(30)));
                        let r = self.linux(ctx, &distro, argv, timeout, false).await?;
                        match r.outcome {
                            CommandOutcome::Exited(0) => match expect_output_contains {
                                Some(x) if !r.tail.join("\n").contains(x.as_str()) => (O::Failed, L::Executable, format!("{executable} ran but its output did not contain '{x}'")),
                                _ => (O::Passed, L::Executable, format!("{executable} ran successfully in {}", distro.name)),
                            },
                            CommandOutcome::Exited(code) => (O::Failed, L::Executable, format!("{executable} exited with code {code}")),
                            CommandOutcome::TimedOut => (O::Failed, L::Executable, format!("{executable} did not finish within {} s", timeout.as_secs())),
                            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
                        }
                    }
                }
            };
            details.push(d);
            if o == O::Passed && (l == L::Executable || level == L::None) {
                level = l;
            }
            outcome = match (outcome, o) {
                (O::Failed, _) | (_, O::Failed) => O::Failed,
                (O::Unavailable, _) | (_, O::Unavailable) => O::Unavailable,
                _ => O::Passed,
            };
        }
        if outcome != O::Passed {
            level = L::None;
        }
        Ok(VerifyReport { outcome, level, detail: details.join("; "), location })
    }

    async fn uninstall(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<InstallOutcome> {
        let pkgs = packages(spec)?;
        let distro = self.resolve_distro(ctx).await?;
        let r = self.linux(ctx, &distro, linux::apt_remove(pkgs), APT_INSTALL_TIMEOUT, false).await?;
        self.expect(r, "apt-get remove")?;
        Ok(InstallOutcome { detail: format!("removed {} from {}. Dependencies were left in place; run 'sudo apt-get autoremove' in the distribution to clean them up.", pkgs.join(", "), distro.name), exit_code: Some(0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::testing::*;
    use crate::domain::manifest::ToolManifest;
    use crate::domain::process::CancellationToken;

    fn manifest() -> ToolManifest {
        ToolManifest::from_toml(include_str!("../../../manifests/nikto.toml")).unwrap()
    }

    const LIST_OK: &[&str] = &["  NAME            STATE           VERSION", "* Ubuntu-24.04    Running         2", "  docker-desktop  Stopped         2"];

    /// A scriptable simulated distribution behind wsl.exe.
    struct Distro {
        installed: std::sync::atomic::AtomicBool,
        list: Vec<&'static str>,
        apt_error: Option<&'static str>,
    }

    fn adapter(d: Arc<Distro>, configured: Option<&str>) -> (WslAptAdapter, Arc<MockExecutor>) {
        let exec = Arc::new(MockExecutor::new(move |c| {
            let a = &c.args;
            if a[0] == "--list" {
                return if d.list.is_empty() { Step::exit(-1, &["The Windows Subsystem for Linux is not installed. You can install by running 'wsl.exe --install'."]) } else { Step::ok(&d.list) };
            }
            let pos = a.iter().position(|x| x == "--exec").expect("every Linux command uses --exec");
            let argv = &a[pos + 1..];
            // Invariants of every command: explicit distro, root user, explicit working directory.
            assert_eq!(a[..pos].to_vec(), ["--distribution", "Ubuntu-24.04", "--user", "root", "--cd", "/tmp"].map(String::from).to_vec());
            match argv[0].as_str() {
                "/usr/bin/test" => Step::ok(&[]),
                "/usr/bin/dpkg-query" => {
                    if d.installed.load(std::sync::atomic::Ordering::SeqCst) {
                        Step::ok(&["ii |nikto|1:2.1.5-3.1"])
                    } else {
                        Step::exit(1, &["dpkg-query: no packages found matching nikto"])
                    }
                }
                "/usr/bin/env" => match argv[2].as_str() {
                    "/usr/bin/apt-get" if argv[3] == "update" => Step::ok(&["Hit:1 http://archive.ubuntu.com/ubuntu noble InRelease"]),
                    "/usr/bin/apt-get" if argv[3] == "install" => match d.apt_error {
                        Some(e) => Step::exit(100, &[e]),
                        None => {
                            d.installed.store(true, std::sync::atomic::Ordering::SeqCst);
                            Step::ok(&["Setting up nikto (1:2.1.5-3.1) ..."])
                        }
                    },
                    "/usr/bin/apt-get" if argv[3] == "remove" => {
                        d.installed.store(false, std::sync::atomic::Ordering::SeqCst);
                        Step::ok(&["Removing nikto ..."])
                    }
                    other => panic!("unexpected {other}"),
                },
                other => panic!("unexpected linux command {other}"),
            }
        }));
        let mut loc = MockLocator::default();
        loc.add("wsl.exe", "C:\\fake\\wsl.exe");
        (WslAptAdapter::new(exec.clone(), Arc::new(loc), configured.map(String::from)), exec)
    }

    fn distro(list: &[&'static str], apt_error: Option<&'static str>) -> Arc<Distro> {
        Arc::new(Distro { installed: std::sync::atomic::AtomicBool::new(false), list: list.to_vec(), apt_error })
    }

    macro_rules! ctx {
        ($m:expr, $s:expr, $c:expr) => {
            AdapterContext { tool: &$m, cancel: &$c, sink: &$s }
        };
    }

    fn spec(m: &ToolManifest) -> InstallSpec {
        m.install[0].clone()
    }

    #[test]
    fn linux_argument_vectors_are_fixed_and_never_use_a_shell() {
        let p = vec!["nikto".to_string()];
        assert_eq!(linux::apt_install(&p, false), ["/usr/bin/env", "DEBIAN_FRONTEND=noninteractive", "/usr/bin/apt-get", "install", "-y", "--no-install-recommends", "--", "nikto"]);
        assert_eq!(linux::apt_install(&p, true)[6], "--only-upgrade");
        assert_eq!(linux::apt_remove(&p), ["/usr/bin/env", "DEBIAN_FRONTEND=noninteractive", "/usr/bin/apt-get", "remove", "-y", "--", "nikto"]);
        assert_eq!(linux::dpkg_status(&p)[2], "-f=${db:Status-Abbrev}|${Package}|${Version}\\n");
        for argv in [linux::apt_update(), linux::apt_install(&p, false), linux::apt_remove(&p), linux::dpkg_status(&p), linux::has_apt()] {
            assert!(argv[0].starts_with('/'), "programs are absolute: {argv:?}");
            assert!(!argv.iter().any(|a| a == "sh" || a == "bash" || a == "-c" || a == "/bin/sh"));
        }
    }

    #[test]
    fn malicious_package_names_never_reach_wsl() {
        for evil in [vec!["nikto; rm -rf /"], vec!["-o"], vec!["a b"], vec!["$(id)"], vec!["x|y"], vec!["UPPER"], vec![], vec![""]] {
            let spec = InstallSpec::WslApt { packages: evil.iter().map(|s| s.to_string()).collect() };
            assert!(packages(&spec).is_err(), "{evil:?}");
        }
        assert!(packages(&InstallSpec::Winget { package_id: "A.B".into() }).is_err());
        assert!(valid_distro("Ubuntu-24.04") && !valid_distro("Ubuntu 24; calc") && !valid_distro("../x"));
    }

    #[test]
    fn parses_dpkg_rows() {
        let rows: Vec<String> = ["ii |nikto|1:2.1.5-3.1", "un |other|", "rc |gone|1.0", "garbage"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_dpkg_status(&rows), vec![("nikto".to_string(), "1:2.1.5-3.1".to_string())]);
    }

    #[tokio::test]
    async fn install_runs_update_then_install_in_the_default_wsl2_distro_and_tracks_the_version() {
        let m = manifest();
        let d = distro(LIST_OK, None);
        let (a, exec) = adapter(d.clone(), None);
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let pre = a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert!(!pre.already_installed);
        assert!(pre.notes[0].contains("not a security sandbox"));
        a.install(&ctx!(m, sink, cancel), &spec(&m), false).await.unwrap();
        let st = a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert_eq!(st, PresenceStatus::Installed { version: Some("1:2.1.5-3.1".into()) });
        let rep = a.verify(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert_eq!((rep.outcome, rep.level), (VerificationOutcome::Passed, VerificationLevel::PackageRecord));
        assert_eq!(rep.location.as_deref(), Some("WSL: Ubuntu-24.04"));
        // The internal docker-desktop distribution was never used; update came before install.
        let verbs: Vec<String> = exec.calls().iter().filter(|c| c.args.contains(&"/usr/bin/env".to_string())).map(|c| c.args[c.args.iter().position(|x| x == "/usr/bin/apt-get").unwrap() + 1].clone()).collect();
        assert_eq!(verbs, ["update", "install"]);
        assert!(exec.calls().iter().all(|c| !c.args.contains(&"docker-desktop".to_string())));
        assert!(sink.lines().iter().any(|l| l.starts_with("> wsl.exe --distribution Ubuntu-24.04 --user root --cd /tmp --exec")));
        // Cleanup removes only the named package.
        let un = a.uninstall(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert!(un.detail.contains("autoremove"));
        assert_eq!(a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap(), PresenceStatus::NotInstalled);
    }

    #[tokio::test]
    async fn missing_wsl_missing_distribution_and_wsl1_only_are_diagnosed_not_guessed() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let kind = |e: AppError| match e {
            AppError::Platform(f) => (f.kind, f.layer),
            other => panic!("{other:?}"),
        };
        // WSL not installed at all.
        let (a, exec) = adapter(distro(&[], None), None);
        let e = a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap_err();
        assert_eq!(kind(e), (FailureKind::WslUnavailable, FailureLayer::Wsl));
        assert_eq!(exec.calls().len(), 1, "nothing else may run once WSL is known to be missing");
        // Status degrades to Unknown instead of "not installed".
        assert!(matches!(a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap(), PresenceStatus::Unknown(_)));
        // Only Docker's utility distribution.
        let (a, _) = adapter(distro(&["  NAME STATE VERSION", "  docker-desktop Stopped 2"], None), None);
        assert_eq!(kind(a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap_err()).0, FailureKind::DistributionMissing);
        // Only a WSL 1 distribution: refused, and the reason says why.
        let (a, _) = adapter(distro(&["  NAME STATE VERSION", "* Ubuntu-24.04 Running 1"], None), None);
        let e = a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap_err();
        assert!(e.to_string().contains("WSL 1"), "{e}");
        // Configured distribution that does not exist.
        let (a, _) = adapter(distro(LIST_OK, None), Some("Debian"));
        let e = a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap_err();
        assert!(e.to_string().contains("'Debian'"));
    }

    #[tokio::test]
    async fn package_manager_errors_are_attributed_to_the_package_manager_not_wsl() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        for (out, kind) in [
            ("E: Unable to locate package nikto", FailureKind::PackageManagerError),
            ("E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 42", FailureKind::PackageManagerError),
            ("E: Failed to fetch http://archive.ubuntu.com/ubuntu/pool/x.deb  Temporary failure resolving 'archive.ubuntu.com'", FailureKind::PackageDownloadFailed),
            ("E: You don't have enough free space in /var/cache/apt/archives/. No space left on device", FailureKind::InsufficientDisk),
            ("E: Failed to fetch  407  Proxy Authentication Required", FailureKind::ProxyRestricted),
        ] {
            let (a, _) = adapter(distro(LIST_OK, Some(out)), None);
            let e = a.install(&ctx!(m, sink, cancel), &spec(&m), false).await.unwrap_err();
            let AppError::Platform(f) = e else { panic!("{out}") };
            assert_eq!(f.kind, kind, "{out}");
            assert_eq!(f.layer, FailureLayer::PackageManager, "{out}");
        }
        // An unrecognised apt failure is still a package-manager command failure with its exit code.
        let (a, _) = adapter(distro(LIST_OK, Some("something odd")), None);
        let e = a.install(&ctx!(m, sink, cancel), &spec(&m), false).await.unwrap_err();
        assert!(matches!(e, AppError::Platform(ref f) if f.layer == FailureLayer::PackageManager), "{e:?}");
    }

    #[tokio::test]
    async fn virtualization_errors_from_wsl_itself_are_reported_as_windows_layer() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let exec = Arc::new(MockExecutor::new(|c| {
            if c.args[0] == "--list" {
                Step::ok(LIST_OK)
            } else {
                Step::exit(-1, &["WSL2 is not supported with your current machine configuration.", "Error code: Wsl/Service/CreateInstance/CreateVm/HCS/HCS_E_HYPERV_NOT_INSTALLED"])
            }
        }));
        let mut loc = MockLocator::default();
        loc.add("wsl.exe", "C:\\fake\\wsl.exe");
        let a = WslAptAdapter::new(exec, Arc::new(loc), None);
        let AppError::Platform(f) = a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap_err() else { panic!() };
        assert_eq!((f.kind, f.layer), (FailureKind::VirtualizationDisabled, FailureLayer::Windows));
    }

    #[tokio::test]
    async fn a_stopped_distribution_is_started_by_the_first_command_and_noted() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let list = &["  NAME STATE VERSION", "* Ubuntu-24.04 Stopped 2"];
        let (a, _) = adapter(distro(list, None), None);
        let pre = a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert!(pre.notes.iter().any(|n| n.contains("stopped and will be started")));
    }

    #[tokio::test]
    async fn cancellation_before_any_command_runs_nothing() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        cancel.cancel();
        let (a, exec) = adapter(distro(LIST_OK, None), None);
        assert!(matches!(a.install(&ctx!(m, sink, cancel), &spec(&m), false).await, Err(AppError::Cancelled)));
        assert!(exec.calls().is_empty());
    }
}

