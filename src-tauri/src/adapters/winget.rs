//! Windows Package Manager (winget) adapter.
//!
//! Only `InstallSpec::Winget` is accepted. The program is the absolute path of `winget.exe`; every
//! operation is a fixed argument array whose only variable parts are the validated package id and
//! optional pinned version. Nothing is ever passed through a shell.
//!
//! Integrity: winget downloads the installer and checks its SHA-256 against the community
//! manifest. Vixalune does **not** re-check it; the manifest's `integrity.level` says so.

use crate::domain::jobs::{VerificationLevel, VerificationOutcome};
use crate::domain::manifest::{InstallMethod, InstallSpec, ToolManifest, VerifyCheck};
use crate::domain::paths;
use crate::domain::process::OutputStream;
use crate::domain::{AppError, AppResult};
use crate::ports::{
    AdapterContext, CommandExecutor, CommandOutcome, CommandResult, CommandSpec, ExecutableLocator, InstallOutcome, InstallerAdapter,
    LineSink, PresenceStatus, PreflightReport, VerifyReport,
};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// winget exit code for "No installed package found matching input criteria" (observed on Windows 11).
pub const WINGET_NO_INSTALLED_PACKAGE: i32 = 0x8A15_0014_u32 as i32;

const READ_TIMEOUT: Duration = Duration::from_secs(90);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DEFAULT_VERIFY_TIMEOUT: u32 = 30;

pub struct WingetAdapter {
    executor: Arc<dyn CommandExecutor>,
    locator: Arc<dyn ExecutableLocator>,
}

struct DiscardSink;
impl LineSink for DiscardSink {
    fn line(&self, _: OutputStream, _: &str) {}
}

fn package_id(spec: &InstallSpec) -> AppResult<&str> {
    match spec {
        InstallSpec::Winget { package_id } => {
            // Defense in depth: manifests are validated on load, but the adapter re-checks.
            let ok = !package_id.is_empty()
                && package_id.len() <= 128
                && !package_id.starts_with('-')
                && package_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
            if ok {
                Ok(package_id)
            } else {
                Err(AppError::Validation(format!("refusing unsafe winget package id '{package_id}'")))
            }
        }
        _ => Err(AppError::Unsupported("the winget adapter only handles winget packages".into())),
    }
}

fn pinned(tool: &ToolManifest) -> AppResult<Option<&str>> {
    match tool.version.pinned.as_deref() {
        None => Ok(None),
        Some(v) if !v.is_empty() && !v.starts_with('-') && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')) => Ok(Some(v)),
        Some(v) => Err(AppError::Validation(format!("refusing unsafe pinned version '{v}'"))),
    }
}

/// The fixed argument arrays. Public for tests and for the audit trail.
pub mod args {
    pub fn show(id: &str) -> Vec<String> {
        ["show", "--id", id, "--exact", "--source", "winget", "--disable-interactivity"].map(String::from).to_vec()
    }
    pub fn list(id: &str) -> Vec<String> {
        ["list", "--id", id, "--exact", "--disable-interactivity"].map(String::from).to_vec()
    }
    pub fn install(id: &str, version: Option<&str>, upgrade: bool) -> Vec<String> {
        let mut a: Vec<String> = [if upgrade { "upgrade" } else { "install" }, "--id", id, "--exact", "--source", "winget"]
            .map(String::from)
            .to_vec();
        if let Some(v) = version {
            a.extend(["--version".to_string(), v.to_string()]);
        }
        a.extend(["--silent", "--disable-interactivity", "--accept-package-agreements"].map(String::from));
        a
    }
    pub fn uninstall(id: &str) -> Vec<String> {
        ["uninstall", "--id", id, "--exact", "--silent", "--disable-interactivity"].map(String::from).to_vec()
    }
}

fn hex(code: i32) -> String {
    format!("0x{:08X}", code as u32)
}

/// `Version: 7.80` from `winget show`.
fn parse_show_version(lines: &[String]) -> Option<String> {
    lines.iter().find_map(|l| l.strip_prefix("Version:").map(|v| v.trim().to_string())).filter(|v| !v.is_empty())
}

/// From a `winget list` table row: the token after the (exact) package id is the version.
fn parse_list_version(lines: &[String], id: &str) -> Option<String> {
    lines.iter().find_map(|l| {
        let mut it = l.split_whitespace();
        it.find(|t| t.eq_ignore_ascii_case(id)).and_then(|_| it.next()).map(|v| v.trim_start_matches(['<', '>']).to_string())
    })
}

fn last_lines(tail: &[String]) -> String {
    let start = tail.len().saturating_sub(8);
    tail[start..].join(" | ")
}

impl WingetAdapter {
    pub fn new(executor: Arc<dyn CommandExecutor>, locator: Arc<dyn ExecutableLocator>) -> Self {
        Self { executor, locator }
    }

    fn winget(&self) -> AppResult<PathBuf> {
        self.locator
            .locate("winget.exe", &[])
            .ok_or_else(|| AppError::PrerequisiteMissing("winget (Windows Package Manager) was not found".into()))
    }

    async fn run(&self, ctx: &AdapterContext<'_>, args: Vec<String>, timeout: Duration, quiet: bool) -> AppResult<CommandResult> {
        if ctx.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let spec = CommandSpec { program: self.winget()?, args, timeout };
        if !quiet {
            // Audit trail: the exact arguments (fixed, validated, no secrets) go into the job output.
            ctx.sink.line(OutputStream::Stdout, &format!("> winget {}", spec.args.join(" ")));
        }
        let discard = DiscardSink;
        let sink: &dyn LineSink = if quiet { &discard } else { ctx.sink };
        self.executor.run(spec, sink, ctx.cancel).await
    }

    fn expect_exit(result: CommandResult, what: &str) -> AppResult<CommandResult> {
        match result.outcome {
            CommandOutcome::Exited(0) => Ok(result),
            CommandOutcome::Exited(code) => Err(AppError::CommandFailed {
                code: Some(code),
                message: format!("winget {what} exited with {} ({code}). {}", hex(code), last_lines(&result.tail)),
            }),
            CommandOutcome::TimedOut => Err(AppError::TimedOut(format!("winget {what} did not finish in time"))),
            CommandOutcome::Cancelled => Err(AppError::Cancelled),
        }
    }

    async fn run_verify_check(&self, ctx: &AdapterContext<'_>, check: &VerifyCheck, spec: &InstallSpec) -> AppResult<(VerificationOutcome, VerificationLevel, String, Option<String>)> {
        use VerificationLevel as L;
        use VerificationOutcome as O;
        match check {
            VerifyCheck::PackageInstalled => Ok(match self.status(ctx, spec).await? {
                PresenceStatus::Installed { version } => {
                    (O::Passed, L::PackageRecord, format!("winget reports the package installed{}", version.map(|v| format!(" (version {v})")).unwrap_or_default()), None)
                }
                PresenceStatus::NotInstalled => (O::Failed, L::PackageRecord, "winget does not list the package as installed".into(), None),
                PresenceStatus::Unknown(why) => (O::Unavailable, L::None, format!("package record could not be checked: {why}"), None),
            }),
            VerifyCheck::ExecutableOutput { executable, args, locations, expect_output_contains, .. } => {
                let mut extra = Vec::new();
                for l in locations {
                    if let Some(p) = paths::resolve_location(l)? {
                        extra.push(p);
                    }
                }
                let Some(exe) = self.locator.locate(executable, &extra) else {
                    return Ok((O::Failed, L::None, format!("{executable} was not found on PATH or in the declared install locations"), None));
                };
                let timeout = Duration::from_secs(u64::from(ctx.tool.verify.timeout_secs.unwrap_or(DEFAULT_VERIFY_TIMEOUT)));
                let location = exe.parent().map(|p| p.display().to_string());
                let result = self.executor.run(CommandSpec { program: exe.clone(), args: args.clone(), timeout }, ctx.sink, ctx.cancel).await?;
                Ok(match result.outcome {
                    CommandOutcome::Exited(0) => {
                        let text = result.tail.join("\n");
                        match expect_output_contains {
                            Some(x) if !text.contains(x.as_str()) => {
                                (O::Failed, L::Executable, format!("{executable} ran but its output did not contain '{x}'"), location)
                            }
                            _ => (O::Passed, L::Executable, format!("{executable} ran successfully"), location),
                        }
                    }
                    CommandOutcome::Exited(code) => (O::Failed, L::Executable, format!("{executable} exited with {} ({code})", hex(code)), location),
                    CommandOutcome::TimedOut => (O::Failed, L::Executable, format!("{executable} did not finish within {} s", timeout.as_secs()), location),
                    CommandOutcome::Cancelled => return Err(AppError::Cancelled),
                })
            }
        }
    }
}

#[async_trait]
impl InstallerAdapter for WingetAdapter {
    fn method(&self) -> InstallMethod {
        InstallMethod::Winget
    }

    fn supports(&self, spec: &InstallSpec) -> bool {
        matches!(spec, InstallSpec::Winget { .. })
    }

    async fn preflight(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PreflightReport> {
        let id = package_id(spec)?;
        pinned(ctx.tool)?;
        let show = self.run(ctx, args::show(id), READ_TIMEOUT, true).await?;
        let show = match show.outcome {
            CommandOutcome::Exited(0) => show,
            CommandOutcome::Exited(code) => {
                return Err(AppError::Unsupported(format!(
                    "winget cannot find package '{id}' in its community source ({}). {}",
                    hex(code),
                    last_lines(&show.tail)
                )))
            }
            CommandOutcome::TimedOut => return Err(AppError::TimedOut("winget could not reach its package source".into())),
            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
        };
        let (installed, installed_version) = match self.status(ctx, spec).await? {
            PresenceStatus::Installed { version } => (true, version),
            _ => (false, None),
        };
        Ok(PreflightReport {
            already_installed: installed,
            installed_version,
            available_version: parse_show_version(&show.tail),
            notes: vec![],
        })
    }

    async fn install(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec, update: bool) -> AppResult<InstallOutcome> {
        let id = package_id(spec)?;
        let version = pinned(ctx.tool)?;
        let what = if update { "upgrade" } else { "install" };
        let r = self.run(ctx, args::install(id, version, update), INSTALL_TIMEOUT, false).await?;
        Self::expect_exit(r, what)?;
        Ok(InstallOutcome { detail: format!("winget {what} of {id} finished"), exit_code: Some(0) })
    }

    async fn status(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PresenceStatus> {
        let id = package_id(spec)?;
        let r = match self.run(ctx, args::list(id), READ_TIMEOUT, true).await {
            Ok(r) => r,
            Err(AppError::PrerequisiteMissing(m)) => return Ok(PresenceStatus::Unknown(m)),
            Err(e) => return Err(e),
        };
        Ok(match r.outcome {
            CommandOutcome::Exited(0) => PresenceStatus::Installed { version: parse_list_version(&r.tail, id) },
            CommandOutcome::Exited(WINGET_NO_INSTALLED_PACKAGE) => PresenceStatus::NotInstalled,
            CommandOutcome::Exited(code) => PresenceStatus::Unknown(format!("winget list exited with {}", hex(code))),
            CommandOutcome::TimedOut => PresenceStatus::Unknown("winget list timed out".into()),
            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
        })
    }

    async fn verify(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<VerifyReport> {
        use VerificationLevel as L;
        use VerificationOutcome as O;
        let mut outcome = O::Passed;
        let mut level = L::None;
        let mut details = Vec::new();
        let mut location = None;
        for check in &ctx.tool.verify.checks {
            let (o, l, d, loc) = self.run_verify_check(ctx, check, spec).await?;
            details.push(d);
            location = location.or(loc);
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
        let id = package_id(spec)?;
        let r = self.run(ctx, args::uninstall(id), INSTALL_TIMEOUT, false).await?;
        Self::expect_exit(r, "uninstall")?;
        Ok(InstallOutcome { detail: format!("winget uninstall of {id} finished"), exit_code: Some(0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::testing::*;
    use crate::domain::manifest::fixtures::VALID;
    use crate::domain::process::CancellationToken;

    fn manifest() -> ToolManifest {
        ToolManifest::from_toml(VALID).unwrap()
    }

    fn adapter(exec: Arc<MockExecutor>, with_winget: bool) -> WingetAdapter {
        let mut loc = MockLocator::default();
        if with_winget {
            loc.add("winget.exe", "C:\\fake\\winget.exe");
        }
        loc.add("nmap.exe", "C:\\Program Files (x86)\\Nmap\\nmap.exe");
        WingetAdapter::new(exec, Arc::new(loc))
    }

    fn spec(m: &ToolManifest) -> InstallSpec {
        m.install[0].clone()
    }

    macro_rules! ctx {
        ($m:expr, $sink:expr, $cancel:expr) => {
            AdapterContext { tool: &$m, cancel: &$cancel, sink: &$sink }
        };
    }

    #[test]
    fn argument_arrays_are_fixed_and_separate_each_value() {
        assert_eq!(
            args::install("Insecure.Nmap", None, false),
            ["install", "--id", "Insecure.Nmap", "--exact", "--source", "winget", "--silent", "--disable-interactivity", "--accept-package-agreements"]
        );
        assert_eq!(
            args::install("A.B", Some("1.2.3"), true),
            ["upgrade", "--id", "A.B", "--exact", "--source", "winget", "--version", "1.2.3", "--silent", "--disable-interactivity", "--accept-package-agreements"]
        );
        assert_eq!(args::uninstall("A.B"), ["uninstall", "--id", "A.B", "--exact", "--silent", "--disable-interactivity"]);
    }

    #[test]
    fn malicious_package_ids_never_reach_a_command() {
        for evil in ["-s", "A.B; calc", "A.B && del x", "A B", "A|B", "A\nB", "", "$(x)", "A.B\" --source x"] {
            let bad = InstallSpec::Winget { package_id: evil.into() };
            assert!(package_id(&bad).is_err(), "{evil:?}");
        }
        assert!(package_id(&InstallSpec::Winget { package_id: "Insecure.Nmap".into() }).is_ok());
        assert!(package_id(&InstallSpec::WslApt { packages: vec!["nmap".into()] }).is_err());
    }

    #[test]
    fn unsafe_pinned_versions_are_refused() {
        let mut m = manifest();
        m.version.pinned = Some("1.0; calc".into());
        assert!(pinned(&m).is_err());
        m.version.pinned = Some("--source".into());
        assert!(pinned(&m).is_err());
        m.version.pinned = Some("7.80".into());
        assert_eq!(pinned(&m).unwrap(), Some("7.80"));
    }

    #[test]
    fn parses_winget_output() {
        let show = vec!["Found Nmap [Insecure.Nmap]".to_string(), "Version: 7.80".to_string()];
        assert_eq!(parse_show_version(&show).as_deref(), Some("7.80"));
        let list = vec![
            "Name  Id            Version Source".to_string(),
            "Nmap  Insecure.Nmap 7.80    winget".to_string(),
        ];
        assert_eq!(parse_list_version(&list, "Insecure.Nmap").as_deref(), Some("7.80"));
        assert_eq!(parse_list_version(&list, "Other.Thing"), None);
    }

    #[tokio::test]
    async fn install_uses_the_absolute_winget_path_and_fixed_args() {
        let exec = Arc::new(MockExecutor::new(|_| Step::ok(&["Successfully installed"])));
        let a = adapter(exec.clone(), true);
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let out = a.install(&ctx!(m, sink, cancel), &spec(&m), false).await.unwrap();
        assert_eq!(out.exit_code, Some(0));
        let calls = exec.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, PathBuf::from("C:\\fake\\winget.exe"));
        assert_eq!(calls[0].args, args::install("Insecure.Nmap", None, false));
        assert!(sink.lines().iter().any(|l| l.contains("Successfully installed")));
    }

    #[tokio::test]
    async fn unexpected_exit_codes_become_actionable_errors() {
        let exec = Arc::new(MockExecutor::new(|_| Step::exit(0x8A15_0010_u32 as i32, &["Installer failed", "Error 1603"])));
        let a = adapter(exec, true);
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let err = a.install(&ctx!(m, sink, cancel), &spec(&m), false).await.unwrap_err();
        match err {
            AppError::CommandFailed { code, message } => {
                assert_eq!(code, Some(0x8A15_0010_u32 as i32));
                assert!(message.contains("0x8A150010") && message.contains("Error 1603"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_winget_is_a_prerequisite_error_and_runs_nothing() {
        let exec = Arc::new(MockExecutor::new(|_| Step::ok(&[])));
        let a = adapter(exec.clone(), false);
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        assert!(matches!(a.install(&ctx!(m, sink, cancel), &spec(&m), false).await, Err(AppError::PrerequisiteMissing(_))));
        assert!(exec.calls().is_empty());
        // status degrades to Unknown rather than claiming "not installed".
        assert!(matches!(a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap(), PresenceStatus::Unknown(_)));
    }

    #[tokio::test]
    async fn timeouts_and_cancellation_are_distinct_errors() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let a = adapter(Arc::new(MockExecutor::new(|_| Step::timed_out())), true);
        assert!(matches!(a.install(&ctx!(m, sink, cancel), &spec(&m), false).await, Err(AppError::TimedOut(_))));
        let a = adapter(Arc::new(MockExecutor::new(|_| Step::cancelled())), true);
        assert!(matches!(a.install(&ctx!(m, sink, cancel), &spec(&m), false).await, Err(AppError::Cancelled)));
        // A token cancelled before the call prevents any process from starting.
        let exec = Arc::new(MockExecutor::new(|_| Step::ok(&[])));
        let a = adapter(exec.clone(), true);
        cancel.cancel();
        assert!(matches!(a.install(&ctx!(m, sink, cancel), &spec(&m), false).await, Err(AppError::Cancelled)));
        assert!(exec.calls().is_empty());
    }

    #[tokio::test]
    async fn status_distinguishes_installed_not_installed_and_unknown() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let a = adapter(Arc::new(MockExecutor::new(|_| Step::ok(&["Nmap Insecure.Nmap 7.80 winget"]))), true);
        assert_eq!(a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap(), PresenceStatus::Installed { version: Some("7.80".into()) });
        let a = adapter(Arc::new(MockExecutor::new(|_| Step::exit(WINGET_NO_INSTALLED_PACKAGE, &["No installed package found"]))), true);
        assert_eq!(a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap(), PresenceStatus::NotInstalled);
        let a = adapter(Arc::new(MockExecutor::new(|_| Step::exit(1, &[]))), true);
        assert!(matches!(a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap(), PresenceStatus::Unknown(_)));
        let a = adapter(Arc::new(MockExecutor::new(|_| Step::timed_out())), true);
        assert!(matches!(a.status(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap(), PresenceStatus::Unknown(_)));
    }

    #[tokio::test]
    async fn preflight_reports_missing_package_and_existing_install() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let a = adapter(Arc::new(MockExecutor::new(|_| Step::exit(1, &["No package found"]))), true);
        assert!(matches!(a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await, Err(AppError::Unsupported(_))));
        let a = adapter(
            Arc::new(MockExecutor::new(|c| {
                if c.args[0] == "show" {
                    Step::ok(&["Found Nmap [Insecure.Nmap]", "Version: 7.99"])
                } else {
                    Step::ok(&["Nmap Insecure.Nmap 7.80 winget"])
                }
            })),
            true,
        );
        let r = a.preflight(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert!(r.already_installed);
        assert_eq!(r.available_version.as_deref(), Some("7.99"));
        assert_eq!(r.installed_version.as_deref(), Some("7.80"));
    }

    fn verify_exec(exe_exit: i32, exe_out: &'static str, listed: bool) -> Arc<MockExecutor> {
        Arc::new(MockExecutor::new(move |c| {
            if c.program.ends_with("winget.exe") {
                if listed {
                    Step::ok(&["Nmap Insecure.Nmap 7.80 winget"])
                } else {
                    Step::exit(WINGET_NO_INSTALLED_PACKAGE, &[])
                }
            } else {
                Step::exit(exe_exit, &[exe_out])
            }
        }))
    }

    #[tokio::test]
    async fn verification_passes_only_when_every_check_passes() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let a = adapter(verify_exec(0, "Nmap version 7.80", true), true);
        let r = a.verify(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert_eq!((r.outcome, r.level), (VerificationOutcome::Passed, VerificationLevel::Executable));
        assert_eq!(r.location.as_deref(), Some("C:\\Program Files (x86)\\Nmap"));

        // Wrong output, non-zero exit, and package not listed each fail verification.
        for (code, out, listed) in [(0, "something else", true), (3, "Nmap version 7.80", true), (0, "Nmap version 7.80", false)] {
            let a = adapter(verify_exec(code, out, listed), true);
            let r = a.verify(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
            assert_eq!(r.outcome, VerificationOutcome::Failed, "{code} {out} {listed}");
            assert_eq!(r.level, VerificationLevel::None);
        }
    }

    #[tokio::test]
    async fn missing_executable_fails_and_unreadable_package_record_is_unavailable() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        // Locator knows winget but not nmap.exe.
        let mut loc = MockLocator::default();
        loc.add("winget.exe", "C:\\fake\\winget.exe");
        let a = WingetAdapter::new(verify_exec(0, "Nmap version", true), Arc::new(loc));
        let r = a.verify(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert_eq!(r.outcome, VerificationOutcome::Failed);
        assert!(r.detail.contains("not found"));

        // winget cannot be queried: package check is Unavailable, which is never Passed.
        let a = adapter(Arc::new(MockExecutor::new(|c| if c.program.ends_with("winget.exe") { Step::timed_out() } else { Step::ok(&["Nmap version 7.80"]) })), true);
        let r = a.verify(&ctx!(m, sink, cancel), &spec(&m)).await.unwrap();
        assert_eq!(r.outcome, VerificationOutcome::Unavailable);
        assert_ne!(r.outcome, VerificationOutcome::Passed);
    }
}
