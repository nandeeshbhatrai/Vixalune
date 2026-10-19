//! End-to-end workflows through the REAL engine, WSL adapter, container adapter, container runner and
//! SQLite store, against a simulated `wsl.exe` / `docker.exe`. These are the automated counterparts of
//! docs/MANUAL_INTEGRATION_TESTS.md: they prove the orchestration, state, diagnosis and cleanup; only the
//! manual run on a machine with WSL 2 / a running Docker engine proves the external programs behave.

use super::*;
use crate::adapters::activity_sqlite::SqliteActivityLogger;
use crate::adapters::container::ContainerAdapter;
use crate::adapters::container_runner::ContainerToolRunner;
use crate::adapters::registry::ManifestRegistry;
use crate::adapters::resolver::BasicDependencyResolver;
use crate::adapters::sqlite_store::SqliteJobStore;
use crate::adapters::testing::{MockExecutor, MockLocator, Step};
use crate::adapters::wsl::WslAptAdapter;
use crate::domain::container::RunApprovals;
use crate::domain::environment::{ComponentState, ComponentStatus, EnvironmentReport};
use crate::domain::manifest::Prerequisite;
use crate::ports::{CommandSpec, ToolRunner, TrustLevel};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

const WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct World {
    // Simulated Linux distribution (nikto package state)
    distro_list: Mutex<Vec<&'static str>>,
    nikto_installed: AtomicBool,
    apt_error: Mutex<Option<&'static str>>,
    wsl_error: Mutex<Option<&'static str>>,
    // Simulated container engine
    docker_up: AtomicBool,
    image_present: AtomicBool,
    pull_error: Mutex<Option<&'static str>>,
    run_step: Mutex<Option<Step>>,
}

fn env(wsl: bool, docker: bool, disk_gb: u64) -> EnvironmentReport {
    let st = |ok: bool| if ok { ComponentState::Available { version: None } } else { ComponentState::Missing };
    EnvironmentReport {
        os: "windows".into(),
        supported_os: true,
        windows_build: Some(26100),
        architecture: "x64".into(),
        free_disk_bytes: Some(disk_gb << 30),
        components: vec![
            ComponentStatus { component: Prerequisite::Winget, state: st(true) },
            ComponentStatus { component: Prerequisite::Wsl2, state: st(wsl) },
            ComponentStatus { component: Prerequisite::Docker, state: st(docker) },
        ],
        issues: vec![],
    }
}

struct Detect(EnvironmentReport);
#[async_trait]
impl EnvironmentDetector for Detect {
    async fn detect(&self) -> EnvironmentReport {
        self.0.clone()
    }
}

fn handler(w: Arc<World>) -> impl Fn(&CommandSpec) -> Step + Send + Sync + 'static {
    move |c: &CommandSpec| {
        let name = c.program.file_name().unwrap().to_string_lossy().to_string();
        let a = &c.args;
        if name == "wsl.exe" {
            if let Some(e) = *w.wsl_error.lock().unwrap() {
                return Step::exit(-1, &[e]);
            }
            if a[0] == "--list" {
                let rows = w.distro_list.lock().unwrap().clone();
                return if rows.is_empty() { Step::exit(-1, &["The Windows Subsystem for Linux is not installed. You can install by running 'wsl.exe --install'."]) } else { Step::ok(&rows) };
            }
            let pos = a.iter().position(|x| x == "--exec").unwrap();
            let argv = &a[pos + 1..];
            return match argv[0].as_str() {
                "/usr/bin/test" => Step::ok(&[]),
                "/usr/bin/dpkg-query" => {
                    if w.nikto_installed.load(Ordering::SeqCst) { Step::ok(&["ii |nikto|1:2.1.5-3.1"]) } else { Step::exit(1, &["dpkg-query: no packages found matching nikto"]) }
                }
                "/usr/bin/env" if argv[3] == "update" => Step::ok(&["Hit:1 http://archive.ubuntu.com/ubuntu noble InRelease"]),
                "/usr/bin/env" if argv[3] == "install" => match *w.apt_error.lock().unwrap() {
                    Some(e) => Step::exit(100, &[e]),
                    None => {
                        w.nikto_installed.store(true, Ordering::SeqCst);
                        Step::ok(&["Setting up nikto (1:2.1.5-3.1) ..."])
                    }
                },
                "/usr/bin/env" if argv[3] == "remove" => {
                    w.nikto_installed.store(false, Ordering::SeqCst);
                    Step::ok(&["Removing nikto ..."])
                }
                other => panic!("unexpected linux command {other}"),
            };
        }
        // docker.exe
        match a[0].as_str() {
            "version" => {
                if w.docker_up.load(Ordering::SeqCst) {
                    Step::ok(&["client=29.8.2 context=desktop-linux server=29.8.2 linux amd64"])
                } else {
                    Step::exit(1, &["client=29.8.2 context=desktop-linux server=none", "failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; check if the path is correct and if the daemon is running"])
                }
            }
            "context" => Step::ok(&["npipe:////./pipe/dockerDesktopLinuxEngine"]),
            "pull" => match *w.pull_error.lock().unwrap() {
                Some(e) => Step::exit(1, &[e]),
                None => {
                    w.image_present.store(true, Ordering::SeqCst);
                    Step::ok(&["Status: Downloaded newer image"])
                }
            },
            "image" if a[1] == "inspect" => {
                if w.image_present.load(Ordering::SeqCst) { Step::ok(&["sha256:0123456789abcdef0123 linux amd64"]) } else { Step::exit(1, &["Error response from daemon: No such image"]) }
            }
            "image" if a[1] == "rm" => {
                w.image_present.store(false, Ordering::SeqCst);
                Step::ok(&["Untagged"])
            }
            "ps" => Step::ok(&[]),
            "rm" => Step::ok(&[]),
            "run" => w.run_step.lock().unwrap().clone().unwrap_or_else(|| Step::ok(&["1.99.0"])),
            other => panic!("unexpected docker command {other}"),
        }
    }
}

struct Lab {
    engine: InstallEngine,
    runner: ContainerToolRunner,
    store: Arc<SqliteJobStore>,
    w: Arc<World>,
    exec: Arc<MockExecutor>,
    _dir: tempfile::TempDir,
}

fn lab(env: EnvironmentReport) -> Lab {
    let w = Arc::new(World::default());
    *w.distro_list.lock().unwrap() = vec!["  NAME STATE VERSION", "* Ubuntu-24.04 Running 2", "  docker-desktop Stopped 2"];
    w.docker_up.store(true, Ordering::SeqCst);
    let exec = Arc::new(MockExecutor::new(handler(w.clone())));
    let mut loc = MockLocator::default();
    loc.add("wsl.exe", "C:\\Windows\\System32\\wsl.exe");
    loc.add("docker.exe", "C:\\fake\\docker.exe");
    let loc = Arc::new(loc);
    let dir = tempfile::tempdir().unwrap();
    let mdir = dir.path().join("manifests");
    std::fs::create_dir_all(&mdir).unwrap();
    for f in ["nikto", "semgrep"] {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests").join(format!("{f}.toml"));
        std::fs::copy(src, mdir.join(format!("{f}.toml"))).unwrap();
    }
    let registry = Arc::new(ManifestRegistry::load_dir(&mdir, TrustLevel::Bundled).unwrap());
    assert!(registry.rejected.is_empty(), "{:?}", registry.rejected);
    let regs = vec!["docker.io".to_string()];
    let home = Some(PathBuf::from("C:\\Users\\me"));
    let container = Arc::new(ContainerAdapter::new(exec.clone(), loc.clone(), regs.clone(), home.clone()));
    let store = Arc::new(SqliteJobStore::in_memory().unwrap());
    let activity = Arc::new(SqliteActivityLogger::in_memory().unwrap());
    let adapters: Vec<Arc<dyn InstallerAdapter>> = vec![Arc::new(WslAptAdapter::new(exec.clone(), loc.clone(), None)), container.clone()];
    let engine = InstallEngine::new(EngineDeps {
        registry: registry.clone(),
        detector: Arc::new(Detect(env)),
        resolver: Arc::new(BasicDependencyResolver),
        adapters,
        store: store.clone(),
        activity: activity.clone(),
    });
    let runner = ContainerToolRunner::new(registry, exec.clone(), loc, container, store.clone(), activity, regs, home);
    Lab { engine, runner, store, w, exec, _dir: dir }
}

fn nikto() -> ToolId {
    ToolId::new("nikto").unwrap()
}
fn semgrep() -> ToolId {
    ToolId::new("semgrep").unwrap()
}

impl Lab {
    async fn done(&self, op: OperationId) -> Job {
        self.engine.wait_for_terminal(op, WAIT).await.unwrap()
    }
}

// ------------------------------------------------------------------ Linux (WSL) workflow

#[tokio::test]
async fn linux_tool_workflow_install_verify_track_version_and_clean_up() {
    let l = lab(env(true, true, 100));
    let job = l.done(l.engine.install(&nikto(), false).await.unwrap()).await;
    assert_eq!(job.phase, JobPhase::Succeeded, "{job:?}");

    let rec = l.store.get_installation(&nikto()).unwrap().unwrap();
    assert_eq!(rec.adapter, InstallMethod::WslApt);
    assert_eq!(rec.version.as_deref(), Some("1:2.1.5-3.1"), "the version inside the distribution is tracked");
    assert_eq!(rec.location.as_deref(), Some("WSL: Ubuntu-24.04"));
    assert_eq!(rec.last_verification.unwrap().level, VerificationLevel::PackageRecord);
    let installed = l.engine.installed().unwrap();
    assert_eq!(installed[0].runtime, crate::domain::manifest::RuntimeKind::Wsl);

    // Output records the exact wsl.exe commands (explicit distro, root in the distro, working directory, no shell).
    let out = l.engine.output(job.id).unwrap();
    assert!(out.iter().any(|x| x.starts_with("> wsl.exe --distribution Ubuntu-24.04 --user root --cd /tmp --exec /usr/bin/env DEBIAN_FRONTEND=noninteractive /usr/bin/apt-get install")));
    // The plan said WSL is not a sandbox.
    assert!(l.engine.plan(&nikto()).await.unwrap().notes.iter().any(|n| n.contains("not a sandbox")));

    // Documented cleanup.
    let un = l.done(l.engine.uninstall(&nikto(), false).await.unwrap()).await;
    assert_eq!(un.phase, JobPhase::Succeeded);
    assert!(l.store.get_installation(&nikto()).unwrap().is_none());
    assert!(!l.w.nikto_installed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn linux_tool_failures_name_the_failing_layer_and_never_report_success() {
    // WSL missing from the environment: the job fails before running anything.
    let l = lab(env(false, true, 100));
    let job = l.done(l.engine.install(&nikto(), false).await.unwrap()).await;
    assert_eq!(job.error.as_ref().unwrap().code, "prerequisite_missing");
    assert!(job.error.unwrap().message.contains("Nikto is a Linux tool"));
    assert!(l.exec.calls().is_empty());

    // WSL present but virtualization off: a Windows-layer diagnosis from wsl.exe itself.
    let l = lab(env(true, true, 100));
    *l.w.wsl_error.lock().unwrap() = Some("Error code: Wsl/Service/CreateInstance/CreateVm/HCS/HCS_E_HYPERV_NOT_INSTALLED");
    let job = l.done(l.engine.install(&nikto(), false).await.unwrap()).await;
    let err = job.error.unwrap();
    assert_eq!(err.code, "virtualization_disabled");
    assert!(err.recovery.contains("BIOS"));

    // No distribution.
    let l = lab(env(true, true, 100));
    l.w.distro_list.lock().unwrap().clear();
    let job = l.done(l.engine.install(&nikto(), false).await.unwrap()).await;
    assert_eq!(job.error.unwrap().code, "wsl_unavailable");

    // Package-manager failure inside WSL (not a WSL error), and nothing is recorded as installed.
    let l = lab(env(true, true, 100));
    *l.w.apt_error.lock().unwrap() = Some("E: Unable to locate package nikto");
    let job = l.done(l.engine.install(&nikto(), false).await.unwrap()).await;
    assert_eq!(job.phase, JobPhase::Failed);
    let err = job.error.unwrap();
    assert_eq!(err.code, "package_manager_error");
    assert!(err.detail.unwrap().contains("Unable to locate package"));
    assert!(l.store.get_installation(&nikto()).unwrap().is_none());

    // Network trouble inside apt: a download failure, with the proxy advice available separately.
    let l = lab(env(true, true, 100));
    *l.w.apt_error.lock().unwrap() = Some("E: Failed to fetch http://archive.ubuntu.com/x  Temporary failure resolving 'archive.ubuntu.com'");
    assert_eq!(l.done(l.engine.install(&nikto(), false).await.unwrap()).await.error.unwrap().code, "package_download_failed");
}

// ------------------------------------------------------------------ container workflow

#[tokio::test]
async fn container_workflow_pull_by_digest_verify_hardened_run_and_cleanup() {
    let l = lab(env(true, true, 100));
    let job = l.done(l.engine.install(&semgrep(), false).await.unwrap()).await;
    assert_eq!(job.phase, JobPhase::Succeeded, "{job:?}");
    let rec = l.store.get_installation(&semgrep()).unwrap().unwrap();
    assert_eq!(rec.adapter, InstallMethod::ContainerImage);
    assert_eq!(rec.last_verification.unwrap().level, VerificationLevel::Executable, "verified by running the tool in a hardened container");
    let pull = l.exec.calls().into_iter().find(|c| c.args[0] == "pull").unwrap();
    assert!(pull.args[1].starts_with("docker.io/semgrep/semgrep@sha256:"), "{pull:?}");
    assert!(l.engine.plan(&semgrep()).await.unwrap().notes.iter().any(|n| n.contains("not a safe place")));

    // Launch through the runner with explicit approvals; the run is a job and the container is hardened.
    let proj = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("mount-tests").join("workflow");
    std::fs::create_dir_all(&proj).unwrap();
    let approvals = RunApprovals { network: false, mounts: HashMap::from([("source".to_string(), proj.display().to_string())]), ports: HashMap::new() };
    let vals = HashMap::from([("config".to_string(), crate::domain::manifest::ParamValue::Text("/src/.semgrep.yml".into()))]);
    *l.w.run_step.lock().unwrap() = Some(Step::ok(&["Ran 4 rules on 12 files: 1 finding."]));
    let preview = l.runner.preview(&semgrep(), "scan-local", &vals, &approvals).unwrap();
    assert_eq!(preview.container.as_ref().unwrap().network, "none");
    let op = l.runner.start(&semgrep(), "scan-local", &vals, &approvals).await.unwrap();
    let run = l.runner.wait(op, WAIT).await.unwrap();
    assert_eq!(run.phase, JobPhase::Succeeded, "{run:?}");
    assert!(l.store.job_output(op, 10).unwrap().iter().any(|x| x.contains("1 finding")));
    let docker_run = l.exec.calls().into_iter().filter(|c| c.args[0] == "run").last().unwrap();
    assert!(crate::domain::container::assert_hardened(&docker_run.args).is_ok());

    // Cleanup removes the downloaded image.
    let un = l.done(l.engine.uninstall(&semgrep(), false).await.unwrap()).await;
    assert_eq!(un.phase, JobPhase::Succeeded);
    assert!(!l.w.image_present.load(Ordering::SeqCst));
}

#[tokio::test]
async fn container_failures_are_diagnosed_and_never_fall_back_to_something_less_isolated() {
    // Engine stopped: the environment says Docker is not ready, so nothing runs at all.
    let l = lab(env(true, false, 100));
    let job = l.done(l.engine.install(&semgrep(), false).await.unwrap()).await;
    assert_eq!(job.error.unwrap().code, "prerequisite_missing");
    assert!(l.exec.calls().is_empty());

    // Engine stops between the readiness check and the pull: still diagnosed precisely.
    let l = lab(env(true, true, 100));
    l.w.docker_up.store(false, Ordering::SeqCst);
    let job = l.done(l.engine.install(&semgrep(), false).await.unwrap()).await;
    let err = job.error.unwrap();
    assert_eq!(err.code, "container_runtime_stopped");
    assert!(err.recovery.contains("Docker Desktop"));
    assert!(l.exec.calls().iter().all(|c| c.args[0] != "pull" && c.args[0] != "run"));

    // Not enough disk for images.
    let l = lab(env(true, true, 2));
    let job = l.done(l.engine.install(&semgrep(), false).await.unwrap()).await;
    assert_eq!(job.error.unwrap().code, "insufficient_disk_space");
    assert!(l.exec.calls().is_empty());

    // Unsupported architecture, proxy and download failures from the registry.
    for (msg, code) in [
        ("no matching manifest for linux/arm64/v8 in the manifest list entries", "unsupported_architecture"),
        ("Error response from daemon: Get \"https://registry-1.docker.io/v2/\": proxyconnect tcp: dial tcp 10.0.0.1:3128: connect: connection refused", "proxy_restricted"),
        ("Error response from daemon: Get \"https://registry-1.docker.io/v2/\": net/http: TLS handshake timeout", "package_download_failed"),
    ] {
        let l = lab(env(true, true, 100));
        *l.w.pull_error.lock().unwrap() = Some(msg);
        let job = l.done(l.engine.install(&semgrep(), false).await.unwrap()).await;
        assert_eq!(job.phase, JobPhase::Failed);
        assert_eq!(job.error.unwrap().code, code, "{msg}");
        assert!(l.store.get_installation(&semgrep()).unwrap().is_none());
    }

    // A run of a tool that is not installed is refused rather than pulling an image on the fly.
    let l = lab(env(true, true, 100));
    let proj = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("mount-tests").join("workflow");
    std::fs::create_dir_all(&proj).unwrap();
    let approvals = RunApprovals { network: true, mounts: HashMap::from([("source".to_string(), proj.display().to_string())]), ports: HashMap::new() };
    assert!(l.runner.start(&semgrep(), "scan-registry-rules", &HashMap::new(), &approvals).await.is_err());
    assert!(l.exec.calls().iter().all(|c| c.args[0] != "pull" && c.args[0] != "run"));
}

#[tokio::test]
async fn a_tool_cannot_be_run_and_removed_at_the_same_time() {
    let l = lab(env(true, true, 100));
    l.done(l.engine.install(&semgrep(), false).await.unwrap()).await;
    *l.w.run_step.lock().unwrap() = Some(Step::hang(&["scanning"]));
    let proj = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("mount-tests").join("workflow");
    std::fs::create_dir_all(&proj).unwrap();
    let approvals = RunApprovals { network: false, mounts: HashMap::from([("source".to_string(), proj.display().to_string())]), ports: HashMap::new() };
    let vals = HashMap::from([("config".to_string(), crate::domain::manifest::ParamValue::Text("/src/.semgrep.yml".into()))]);
    let op = l.runner.start(&semgrep(), "scan-local", &vals, &approvals).await.unwrap();
    for _ in 0..200 {
        if l.store.get_job(op).unwrap().phase == JobPhase::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(matches!(l.engine.uninstall(&semgrep(), false).await, Err(AppError::Busy(_))));
    assert_eq!(l.engine.states().unwrap().iter().find(|s| s.tool_id == semgrep()).unwrap().state, Some(InstallState::Installed));
    l.runner.cancel(op).await.unwrap();
    assert_eq!(l.runner.wait(op, WAIT).await.unwrap().phase, JobPhase::Cancelled);
    assert_eq!(l.done(l.engine.uninstall(&semgrep(), false).await.unwrap()).await.phase, JobPhase::Succeeded);
}
