//! Container adapter (`InstallSpec::ContainerImage`) over the Docker CLI.
//!
//! * The engine must be **local** and running Linux containers; remote contexts and Windows
//!   containers are refused (no silent fallback to anything less isolated or somewhere else).
//! * Images come only from approved registries, are referenced **by digest only** (never a tag)
//!   and are pulled once; runs use `--pull never`, so nothing is fetched at run time.
//! * Verification runs the tool in the same hardened container used for real runs: non-root,
//!   all capabilities dropped, read-only root, no network, resource limits.
//! * Cleanup removes the image and any container labelled for this tool.
//!
//! A container is not a safe sandbox for hostile code; see `domain::container`.

use crate::adapters::platform_detector::{endpoint_is_local, parse_docker_version};
use crate::domain::container::{assert_hardened, build_docker_run, check_registry, container_name, parse_image, ImageRef, RunApprovals};
use crate::domain::failure::{classify, FailureKind, FailureLayer, OutputSource, PlatformFailure};
use crate::domain::jobs::{VerificationLevel, VerificationOutcome};
use crate::domain::manifest::{ArgTemplate, InstallMethod, InstallSpec, RunSpec, RuntimeKind, ToolManifest, VerifyCheck};
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

const READ_TIMEOUT: Duration = Duration::from_secs(60);
const PULL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct ContainerAdapter {
    executor: Arc<dyn CommandExecutor>,
    locator: Arc<dyn ExecutableLocator>,
    approved_registries: Vec<String>,
    home: Option<PathBuf>,
}

struct Discard;
impl LineSink for Discard {
    fn line(&self, _: OutputStream, _: &str) {}
}

/// Fixed docker argument vectors. Public for tests and for the runner.
pub mod docker {
    pub fn version() -> Vec<String> {
        vec![
            "version".into(),
            "--format".into(),
            "client={{.Client.Version}} context={{.Client.Context}} server={{if .Server}}{{.Server.Version}} {{.Server.Os}} {{.Server.Arch}}{{else}}none{{end}}".into(),
        ]
    }
    pub fn context() -> Vec<String> {
        ["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"].map(String::from).to_vec()
    }
    pub fn pull(reference: &str) -> Vec<String> {
        vec!["pull".into(), reference.into()]
    }
    pub fn inspect(reference: &str) -> Vec<String> {
        vec!["image".into(), "inspect".into(), "--format".into(), "{{.Id}} {{.Os}} {{.Architecture}}".into(), reference.into()]
    }
    pub fn remove_image(reference: &str) -> Vec<String> {
        vec!["image".into(), "rm".into(), reference.into()]
    }
    pub fn list_tool_containers(tool: &str) -> Vec<String> {
        vec!["ps".into(), "-aq".into(), "--filter".into(), format!("label=vixalune.tool={tool}")]
    }
    pub fn force_remove(id: &str) -> Vec<String> {
        vec!["rm".into(), "-f".into(), id.into()]
    }
}

fn valid_container_id(s: &str) -> bool {
    (12..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The image a manifest installs, parsed and checked against the registry allow-list.
pub fn image_for(manifest: &ToolManifest, spec: &InstallSpec, approved: &[String]) -> AppResult<ImageRef> {
    let InstallSpec::ContainerImage { image, digest } = spec else {
        return Err(AppError::Unsupported("the container adapter only handles container images".into()));
    };
    let _ = manifest;
    let img = parse_image(image, digest)?;
    check_registry(&img, approved)?;
    Ok(img)
}

/// Argument vector for a verification command, from the manifest's fixed check arguments.
fn synthetic_profile(executable: &str, args: &[String]) -> RunSpec {
    RunSpec {
        id: "verify".into(),
        label: "Verify".into(),
        description: String::new(),
        runtime: RuntimeKind::Container,
        executable: executable.into(),
        parameters: vec![],
        args: args.iter().map(|a| ArgTemplate::Literal { value: a.clone() }).collect(),
        mounts: vec![],
        needs_network: false,
        ports: vec![],
    }
}

impl ContainerAdapter {
    pub fn new(executor: Arc<dyn CommandExecutor>, locator: Arc<dyn ExecutableLocator>, approved_registries: Vec<String>, home: Option<PathBuf>) -> Self {
        Self { executor, locator, approved_registries, home }
    }

    fn docker_exe(&self) -> AppResult<PathBuf> {
        self.locator.locate("docker.exe", &[]).ok_or_else(|| AppError::PrerequisiteMissing("a container engine (docker.exe) was not found".into()))
    }

    async fn run(&self, ctx: &AdapterContext<'_>, args: Vec<String>, timeout: Duration, quiet: bool) -> AppResult<CommandResult> {
        if ctx.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let spec = CommandSpec { program: self.docker_exe()?, args, timeout };
        if !quiet {
            ctx.sink.line(OutputStream::Stdout, &format!("> docker {}", spec.args.join(" ")));
        }
        let discard = Discard;
        let sink: &dyn LineSink = if quiet { &discard } else { ctx.sink };
        self.executor.run(spec, sink, ctx.cancel).await
    }

    fn expect(&self, r: CommandResult, what: &str) -> AppResult<CommandResult> {
        match r.outcome {
            CommandOutcome::Exited(0) => Ok(r),
            CommandOutcome::Cancelled => Err(AppError::Cancelled),
            CommandOutcome::TimedOut => Err(AppError::TimedOut(format!("{what} did not finish in time"))),
            CommandOutcome::Exited(code) => {
                if let Some(f) = classify(OutputSource::Docker, Some(code), &r.tail) {
                    return Err(AppError::Platform(f));
                }
                let start = r.tail.len().saturating_sub(6);
                Err(AppError::CommandFailed { code: Some(code), message: format!("{what} exited with code {code}. {}", r.tail[start..].join(" | ")) })
            }
        }
    }

    /// The engine must be reachable, local, and running Linux containers. Returns its architecture.
    pub async fn ensure_ready(&self, ctx: &AdapterContext<'_>) -> AppResult<String> {
        let v = self.run(ctx, docker::version(), READ_TIMEOUT, true).await?;
        match v.outcome {
            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
            CommandOutcome::TimedOut => return Err(AppError::TimedOut("the container engine did not answer".into())),
            CommandOutcome::Exited(_) => {}
        }
        let parsed = parse_docker_version(&v.tail);
        let Some((_, os, arch)) = parsed.and_then(|p| p.server) else {
            let reason = v.tail.iter().find(|l| !l.starts_with("client=")).cloned().unwrap_or_else(|| "the engine did not respond".into());
            let f = classify(OutputSource::Docker, None, &v.tail).unwrap_or_else(|| PlatformFailure::new(FailureKind::ContainerRuntimeStopped, FailureLayer::ContainerRuntime, reason));
            return Err(AppError::Platform(f));
        };
        // A DOCKER_HOST override or remote context would run "local" labs on someone else's machine.
        if let Ok(h) = std::env::var("DOCKER_HOST") {
            if !h.trim().is_empty() && !endpoint_is_local(&h) {
                return Err(AppError::Platform(PlatformFailure::new(FailureKind::RemoteContainerContext, FailureLayer::ContainerRuntime, format!("DOCKER_HOST points to {h}"))));
            }
        }
        let c = self.run(ctx, docker::context(), READ_TIMEOUT, true).await?;
        if let (CommandOutcome::Exited(0), Some(endpoint)) = (c.outcome, c.tail.iter().find(|l| !l.trim().is_empty())) {
            if !endpoint_is_local(endpoint) {
                return Err(AppError::Platform(PlatformFailure::new(FailureKind::RemoteContainerContext, FailureLayer::ContainerRuntime, format!("the active Docker context uses {endpoint}"))));
            }
        }
        if os != "linux" {
            return Err(AppError::Platform(PlatformFailure::new(FailureKind::UnsupportedContainerOs, FailureLayer::ContainerRuntime, format!("the engine runs {os} containers"))));
        }
        Ok(arch)
    }

    async fn inspect(&self, ctx: &AdapterContext<'_>, reference: &str) -> AppResult<PresenceStatus> {
        let r = self.run(ctx, docker::inspect(reference), READ_TIMEOUT, true).await?;
        Ok(match r.outcome {
            CommandOutcome::Exited(0) => {
                let line = r.tail.iter().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
                let mut it = line.split_whitespace();
                let id = it.next().unwrap_or("");
                let os = it.next().unwrap_or("");
                if os != "linux" {
                    PresenceStatus::Unknown(format!("the stored image is for '{os}', not linux"))
                } else {
                    PresenceStatus::Installed { version: Some(id.trim_start_matches("sha256:").chars().take(12).collect::<String>()) }
                }
            }
            CommandOutcome::Exited(_) => match classify(OutputSource::Docker, None, &r.tail) {
                Some(f) => PresenceStatus::Unknown(format!("{}: {}", f.title(), f.detail)),
                None => PresenceStatus::NotInstalled,
            },
            CommandOutcome::TimedOut => PresenceStatus::Unknown("docker image inspect timed out".into()),
            CommandOutcome::Cancelled => return Err(AppError::Cancelled),
        })
    }

    /// Run a command in the hardened, offline verification container.
    async fn run_hardened(&self, ctx: &AdapterContext<'_>, image: &ImageRef, executable: &str, args: &[String], timeout: Duration) -> AppResult<CommandResult> {
        let profile = synthetic_profile(executable, args);
        let name = container_name(ctx.tool.id.as_str(), &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let run = build_docker_run(ctx.tool, &profile, args, &image.reference(), &RunApprovals::default(), &name, self.home.as_deref())?;
        assert_hardened(&run.args)?;
        let r = self.run(ctx, run.args, timeout, false).await?;
        if matches!(r.outcome, CommandOutcome::TimedOut | CommandOutcome::Cancelled) {
            // The docker client was killed but the container may still be running.
            let _ = self.run(&AdapterContext { tool: ctx.tool, cancel: &crate::domain::process::CancellationToken::new(), sink: ctx.sink }, docker::force_remove(&name), READ_TIMEOUT, true).await;
        }
        Ok(r)
    }

    /// Remove every container this tool left behind (labelled `vixalune.tool=<id>`).
    pub async fn cleanup_containers(&self, ctx: &AdapterContext<'_>) -> AppResult<usize> {
        let r = self.run(ctx, docker::list_tool_containers(ctx.tool.id.as_str()), READ_TIMEOUT, true).await?;
        let ids: Vec<String> = r.tail.iter().map(|l| l.trim().to_string()).filter(|l| valid_container_id(l)).collect();
        for id in &ids {
            let r = self.run(ctx, docker::force_remove(id), READ_TIMEOUT, false).await?;
            self.expect(r, "docker rm")?;
        }
        Ok(ids.len())
    }
}

#[async_trait]
impl InstallerAdapter for ContainerAdapter {
    fn method(&self) -> InstallMethod {
        InstallMethod::ContainerImage
    }

    fn supports(&self, spec: &InstallSpec) -> bool {
        matches!(spec, InstallSpec::ContainerImage { .. })
    }

    async fn preflight(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PreflightReport> {
        let img = image_for(ctx.tool, spec, &self.approved_registries)?;
        let arch = self.ensure_ready(ctx).await?;
        let mut notes = vec![
            format!("Image {} will be pulled from {} by digest (no tag).", img.repository, img.registry),
            "Vixalune checks the digest the registry returns; it does not verify publisher signatures (cosign/Notary).".to_string(),
            "Containers are not a safe sandbox for hostile code. Tools run as a non-root user, with no capabilities, a read-only root filesystem and no network unless you approve it.".to_string(),
            format!("Container engine architecture: {arch}."),
        ];
        let (installed, version) = match self.inspect(ctx, &img.reference()).await? {
            PresenceStatus::Installed { version } => (true, version),
            _ => (false, None),
        };
        if installed {
            notes.push("The image is already downloaded.".into());
        }
        Ok(PreflightReport { already_installed: installed, installed_version: version, available_version: None, notes })
    }

    async fn install(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec, _update: bool) -> AppResult<InstallOutcome> {
        let img = image_for(ctx.tool, spec, &self.approved_registries)?;
        let reference = img.reference();
        let r = self.run(ctx, docker::pull(&reference), PULL_TIMEOUT, false).await?;
        self.expect(r, "docker pull")?;
        // Content addressing guarantees the bytes match the digest; also insist on a Linux image.
        match self.inspect(ctx, &reference).await? {
            PresenceStatus::Installed { .. } => {}
            PresenceStatus::NotInstalled => return Err(AppError::VerificationFailed("the image is not present after pulling".into())),
            PresenceStatus::Unknown(why) => return Err(AppError::VerificationFailed(why)),
        }
        Ok(InstallOutcome { detail: format!("pulled {reference}"), exit_code: Some(0) })
    }

    async fn status(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<PresenceStatus> {
        let img = image_for(ctx.tool, spec, &self.approved_registries)?;
        match self.inspect(ctx, &img.reference()).await {
            Err(AppError::PrerequisiteMissing(m)) => Ok(PresenceStatus::Unknown(m)),
            other => other,
        }
    }

    async fn verify(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<VerifyReport> {
        use VerificationLevel as L;
        use VerificationOutcome as O;
        let img = image_for(ctx.tool, spec, &self.approved_registries)?;
        let mut outcome = O::Passed;
        let mut level = L::None;
        let mut details = Vec::new();
        for check in &ctx.tool.verify.checks {
            let (o, l, d) = match check {
                VerifyCheck::PackageInstalled => match self.inspect(ctx, &img.reference()).await? {
                    PresenceStatus::Installed { version } => (O::Passed, L::PackageRecord, format!("image {} is present{}", img.digest.chars().take(19).collect::<String>(), version.map(|v| format!(" (id {v})")).unwrap_or_default())),
                    PresenceStatus::NotInstalled => (O::Failed, L::PackageRecord, "the image is not present".into()),
                    PresenceStatus::Unknown(why) => (O::Unavailable, L::None, format!("the image could not be checked: {why}")),
                },
                VerifyCheck::ExecutableOutput { runtime, executable, args, expect_output_contains, .. } => {
                    if *runtime != RuntimeKind::Container {
                        (O::Unavailable, L::None, format!("check for runtime {runtime:?} is not handled by the container adapter"))
                    } else {
                        let timeout = Duration::from_secs(u64::from(ctx.tool.verify.timeout_secs.unwrap_or(60)));
                        match self.run_hardened(ctx, &img, executable, args, timeout).await {
                            Ok(r) => match r.outcome {
                                CommandOutcome::Exited(0) => match expect_output_contains {
                                    Some(x) if !r.tail.join("\n").contains(x.as_str()) => (O::Failed, L::Executable, format!("{executable} ran but its output did not contain '{x}'")),
                                    _ => (O::Passed, L::Executable, format!("{executable} ran successfully in a hardened container")),
                                },
                                CommandOutcome::Exited(code) => (O::Failed, L::Executable, format!("{executable} exited with code {code} in the container")),
                                CommandOutcome::TimedOut => (O::Failed, L::Executable, format!("{executable} did not finish within {} s", timeout.as_secs())),
                                CommandOutcome::Cancelled => return Err(AppError::Cancelled),
                            },
                            Err(AppError::Platform(f)) => (O::Unavailable, L::None, format!("{}: {}", f.title(), f.detail)),
                            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                            Err(e) => (O::Unavailable, L::None, e.to_string()),
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
        Ok(VerifyReport { outcome, level, detail: details.join("; "), location: Some(format!("docker image {}", img.reference())) })
    }

    async fn uninstall(&self, ctx: &AdapterContext<'_>, spec: &InstallSpec) -> AppResult<InstallOutcome> {
        let img = image_for(ctx.tool, spec, &self.approved_registries)?;
        let removed = self.cleanup_containers(ctx).await?;
        let r = self.run(ctx, docker::remove_image(&img.reference()), READ_TIMEOUT, false).await?;
        self.expect(r, "docker image rm")?;
        Ok(InstallOutcome { detail: format!("removed image {} and {removed} leftover container(s). Shared base layers still used by other images are kept by Docker.", img.reference()), exit_code: Some(0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::testing::*;
    use crate::domain::process::CancellationToken;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn manifest() -> ToolManifest {
        ToolManifest::from_toml(include_str!("../../../manifests/semgrep.toml")).unwrap()
    }

    #[derive(Default)]
    struct Engine {
        up: AtomicBool,
        has_image: AtomicBool,
        os: &'static str,
        endpoint: &'static str,
        pull_error: Option<&'static str>,
        run_exit: i32,
    }

    fn adapter(e: Arc<Engine>, registries: &[&str]) -> (ContainerAdapter, Arc<MockExecutor>) {
        let exec = Arc::new(MockExecutor::new(move |c| {
            let a: Vec<&str> = c.args.iter().map(String::as_str).collect();
            match a[0] {
                "version" => {
                    if e.up.load(Ordering::SeqCst) {
                        Step::ok(&[&format!("client=29.8.2 context=desktop-linux server=29.8.2 {} amd64", e.os)])
                    } else {
                        Step::exit(1, &["client=29.8.2 context=desktop-linux server=none", "failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; check if the path is correct and if the daemon is running"])
                    }
                }
                "context" => Step::ok(&[e.endpoint]),
                "pull" => match e.pull_error {
                    Some(m) => Step::exit(1, &[m]),
                    None => {
                        e.has_image.store(true, Ordering::SeqCst);
                        Step::ok(&["Status: Downloaded newer image for semgrep/semgrep"])
                    }
                },
                "image" if a[1] == "inspect" => {
                    if !e.up.load(Ordering::SeqCst) {
                        Step::exit(1, &["failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; check if the path is correct and if the daemon is running"])
                    } else if e.has_image.load(Ordering::SeqCst) {
                        Step::ok(&["sha256:0123456789abcdef0123 linux amd64"])
                    } else {
                        Step::exit(1, &["Error response from daemon: No such image: x"])
                    }
                }
                "image" if a[1] == "rm" => {
                    e.has_image.store(false, Ordering::SeqCst);
                    Step::ok(&["Untagged"])
                }
                "ps" => Step::ok(&["0123456789ab", "not-a-container-id; rm -rf /"]),
                "rm" => Step::ok(&[]),
                "run" => {
                    // Every run is hardened and offline when verifying.
                    assert_hardened(&c.args).expect("verification containers must be hardened");
                    Step::exit(e.run_exit, &["1.99.0"])
                }
                other => panic!("unexpected docker subcommand {other}"),
            }
        }));
        let mut loc = MockLocator::default();
        loc.add("docker.exe", "C:\\fake\\docker.exe");
        (ContainerAdapter::new(exec.clone(), Arc::new(loc), registries.iter().map(|s| s.to_string()).collect(), Some(PathBuf::from("C:\\Users\\me"))), exec)
    }

    fn up() -> Arc<Engine> {
        Arc::new(Engine { up: AtomicBool::new(true), os: "linux", endpoint: "npipe:////./pipe/dockerDesktopLinuxEngine", ..Default::default() })
    }

    macro_rules! ctx {
        ($m:expr, $s:expr, $c:expr) => {
            AdapterContext { tool: &$m, cancel: &$c, sink: &$s }
        };
    }

    const OK_REGS: &[&str] = &["docker.io", "ghcr.io"];

    #[tokio::test]
    async fn pulls_by_digest_verifies_in_a_hardened_container_and_cleans_up() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let (a, exec) = adapter(up(), OK_REGS);
        let spec = m.install[0].clone();
        let pre = a.preflight(&ctx!(m, sink, cancel), &spec).await.unwrap();
        assert!(!pre.already_installed);
        assert!(pre.notes.iter().any(|n| n.contains("does not verify publisher signatures")));
        a.install(&ctx!(m, sink, cancel), &spec, false).await.unwrap();
        let rep = a.verify(&ctx!(m, sink, cancel), &spec).await.unwrap();
        assert_eq!((rep.outcome, rep.level), (VerificationOutcome::Passed, VerificationLevel::Executable));
        // The pull used the digest-only reference.
        let pull = exec.calls().into_iter().find(|c| c.args[0] == "pull").unwrap();
        assert_eq!(pull.args[1], "docker.io/semgrep/semgrep@sha256:529ee8a277ec8adc5b534d7c74eea0a47e9de21d62852b6ba7ac6ba9566845c3");
        // The verification container was hardened, offline and never pulled anything.
        let run = exec.calls().into_iter().find(|c| c.args[0] == "run").unwrap();
        let has = |k: &str, v: &str| run.args.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(has("--network", "none") && has("--cap-drop", "ALL") && has("--pull", "never") && has("--user", "65532:65532"));
        assert!(run.args.contains(&"--read-only".to_string()));
        // Cleanup removes the image and only well-formed container ids.
        let un = a.uninstall(&ctx!(m, sink, cancel), &spec).await.unwrap();
        assert!(un.detail.contains("1 leftover container"));
        let rms: Vec<_> = exec.calls().into_iter().filter(|c| c.args[0] == "rm").collect();
        assert_eq!(rms.len(), 1);
        assert_eq!(rms[0].args, ["rm", "-f", "0123456789ab"]);
        assert_eq!(a.status(&ctx!(m, sink, cancel), &spec).await.unwrap(), PresenceStatus::NotInstalled);
    }

    #[tokio::test]
    async fn images_from_unapproved_registries_are_never_pulled() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let (a, exec) = adapter(up(), &["ghcr.io"]);
        let err = a.install(&ctx!(m, sink, cancel), &m.install[0], false).await.unwrap_err();
        assert!(matches!(err, AppError::Unsupported(_)) && err.to_string().contains("docker.io"), "{err}");
        assert!(a.preflight(&ctx!(m, sink, cancel), &m.install[0]).await.is_err());
        assert!(exec.calls().iter().all(|c| c.args[0] != "pull"));
    }

    #[tokio::test]
    async fn stopped_remote_and_windows_engines_are_refused_with_diagnoses() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let kind = |e: AppError| match e {
            AppError::Platform(f) => f.kind,
            other => panic!("{other:?}"),
        };
        let stopped = Arc::new(Engine { os: "linux", endpoint: "npipe:////./pipe/x", ..Default::default() });
        let (a, exec) = adapter(stopped, OK_REGS);
        assert_eq!(kind(a.preflight(&ctx!(m, sink, cancel), &m.install[0]).await.unwrap_err()), FailureKind::ContainerRuntimeStopped);
        assert!(exec.calls().iter().all(|c| c.args[0] != "pull" && c.args[0] != "run"));
        let remote = Arc::new(Engine { up: AtomicBool::new(true), os: "linux", endpoint: "ssh://me@build-box", ..Default::default() });
        let (a, _) = adapter(remote, OK_REGS);
        assert_eq!(kind(a.preflight(&ctx!(m, sink, cancel), &m.install[0]).await.unwrap_err()), FailureKind::RemoteContainerContext);
        let windows = Arc::new(Engine { up: AtomicBool::new(true), os: "windows", endpoint: "npipe:////./pipe/x", ..Default::default() });
        let (a, _) = adapter(windows, OK_REGS);
        assert_eq!(kind(a.preflight(&ctx!(m, sink, cancel), &m.install[0]).await.unwrap_err()), FailureKind::UnsupportedContainerOs);
        // Status of a tool whose engine is stopped is unknown, never "not installed".
        let stopped = Arc::new(Engine { os: "linux", endpoint: "npipe:////./pipe/x", ..Default::default() });
        let (a, _) = adapter(stopped, OK_REGS);
        assert!(matches!(a.status(&ctx!(m, sink, cancel), &m.install[0]).await.unwrap(), PresenceStatus::Unknown(_)));
    }

    #[tokio::test]
    async fn pull_failures_are_classified() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        for (msg, kind) in [
            ("no matching manifest for linux/arm64/v8 in the manifest list entries", FailureKind::UnsupportedArchitecture),
            ("Error response from daemon: Get \"https://registry-1.docker.io/v2/\": proxyconnect tcp: dial tcp 10.0.0.1:3128: connect: connection refused", FailureKind::ProxyRestricted),
            ("Error response from daemon: Get \"https://registry-1.docker.io/v2/\": net/http: TLS handshake timeout", FailureKind::PackageDownloadFailed),
            ("write /var/lib/docker/tmp/x: no space left on device", FailureKind::InsufficientDisk),
        ] {
            let e = Arc::new(Engine { up: AtomicBool::new(true), os: "linux", endpoint: "npipe:////./pipe/x", pull_error: Some(msg), ..Default::default() });
            let (a, _) = adapter(e, OK_REGS);
            let AppError::Platform(f) = a.install(&ctx!(m, sink, cancel), &m.install[0], false).await.unwrap_err() else { panic!("{msg}") };
            assert_eq!(f.kind, kind, "{msg}");
        }
    }

    #[tokio::test]
    async fn verification_failures_are_never_success() {
        let m = manifest();
        let (sink, cancel) = (VecSink::default(), CancellationToken::new());
        let e = Arc::new(Engine { up: AtomicBool::new(true), has_image: AtomicBool::new(true), os: "linux", endpoint: "npipe:////./pipe/x", run_exit: 3, ..Default::default() });
        let (a, _) = adapter(e, OK_REGS);
        let rep = a.verify(&ctx!(m, sink, cancel), &m.install[0]).await.unwrap();
        assert_eq!(rep.outcome, VerificationOutcome::Failed);
        // Engine stops mid-way: verification is unavailable, not passed.
        let down = Arc::new(Engine { os: "linux", endpoint: "npipe:////./pipe/x", ..Default::default() });
        let (a, _) = adapter(down, OK_REGS);
        let rep = a.verify(&ctx!(m, sink, cancel), &m.install[0]).await.unwrap();
        assert_ne!(rep.outcome, VerificationOutcome::Passed);
    }

    #[test]
    fn docker_argument_vectors_are_fixed() {
        assert_eq!(docker::pull("r/x@sha256:ab"), ["pull", "r/x@sha256:ab"]);
        assert_eq!(docker::remove_image("r/x@sha256:ab"), ["image", "rm", "r/x@sha256:ab"]);
        assert_eq!(docker::list_tool_containers("semgrep"), ["ps", "-aq", "--filter", "label=vixalune.tool=semgrep"]);
        assert!(valid_container_id("0123456789ab") && !valid_container_id("abc") && !valid_container_id("0123456789ab; ls"));
    }
}

