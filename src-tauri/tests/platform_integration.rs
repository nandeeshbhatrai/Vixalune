//! Opt-in READ-ONLY integration tests against this Windows PC's real WSL, Docker and winget.
//! They change nothing. Run explicitly:
//!
//!     cargo test --manifest-path src-tauri/Cargo.toml --test platform_integration -- --ignored --nocapture
//!
//! Assertions are written so they hold on any Windows machine whatever its state: they check that
//! the report is internally consistent and that failures are diagnosed correctly for the state found.

use std::path::PathBuf;
use std::sync::Arc;
use vixalune_lib::adapters::container::ContainerAdapter;
use vixalune_lib::adapters::executor::SystemCommandExecutor;
use vixalune_lib::adapters::host_facts::WindowsHostFacts;
use vixalune_lib::adapters::locator::SystemExecutableLocator;
use vixalune_lib::adapters::platform_detector::SystemPlatformDetector;
use vixalune_lib::adapters::registry::ManifestRegistry;
use vixalune_lib::adapters::wsl::WslAptAdapter;
use vixalune_lib::domain::container::DEFAULT_APPROVED_REGISTRIES;
use vixalune_lib::domain::failure::FailureKind;
use vixalune_lib::domain::platform::{DaemonState, Readiness, SetupActionId, SetupKind};
use vixalune_lib::domain::process::{CancellationToken, OutputStream};
use vixalune_lib::domain::ToolId;
use vixalune_lib::domain::AppError;
use vixalune_lib::ports::{AdapterContext, InstallerAdapter, LineSink, PlatformDetector, ToolRegistry, TrustLevel};

struct Print;
impl LineSink for Print {
    fn line(&self, _: OutputStream, t: &str) {
        println!("    | {t}");
    }
}

fn detector() -> SystemPlatformDetector {
    SystemPlatformDetector::new(
        Arc::new(SystemCommandExecutor),
        Arc::new(SystemExecutableLocator),
        Arc::new(WindowsHostFacts),
        std::env::temp_dir(),
        None,
    )
}

fn registry() -> ManifestRegistry {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests");
    let r = ManifestRegistry::load_dir(&dir, TrustLevel::Bundled).unwrap();
    assert!(r.rejected.is_empty(), "{:?}", r.rejected);
    r
}

#[tokio::test]
#[ignore = "probes the real machine; run with --ignored"]
async fn real_platform_report_is_consistent() {
    let r = detector().detect().await;
    println!("{}", serde_json::to_string_pretty(&r).unwrap());

    assert!(r.windows.build.is_some_and(|b| b >= 19041));
    assert!(matches!(r.windows.architecture.as_str(), "x64" | "arm64"));
    assert!(r.disk.free_bytes.is_some());

    // WSL: if it is absent we must say so, explain how to fix it, and never offer a restart by itself.
    if r.wsl.installed == Some(false) {
        assert_eq!(r.readiness.linux_tools_wsl.state, Readiness::NeedsSetup);
        let wsl = r.setup_actions.iter().find(|a| a.id == SetupActionId::InstallWsl).expect("a WSL install step is proposed");
        assert!(wsl.requires_admin && wsl.requires_restart && wsl.kind == SetupKind::Automatic);
        assert!(wsl.official_link.as_deref().unwrap().starts_with("https://learn.microsoft.com/"));
    }
    if r.wsl.installed == Some(true) {
        println!("WSL distributions: {:?}", r.wsl.distros);
    }

    // Containers: a CLI without a running engine must never count as ready.
    if let Some(rt) = &r.containers.runtime {
        match &rt.daemon {
            DaemonState::Running { os_type, .. } => assert_eq!(r.readiness.containers.state == Readiness::Ready, os_type.as_deref() == Some("linux") && rt.context_local == Some(true)),
            DaemonState::Stopped { .. } => assert_ne!(r.readiness.containers.state, Readiness::Ready),
            DaemonState::Unknown { .. } => {}
        }
    } else {
        assert_eq!(r.readiness.containers.state, Readiness::NeedsSetup);
    }

    // Permission is separate from detection: only WSL/distro steps are ever automatic.
    for a in &r.setup_actions {
        if a.kind == SetupKind::Automatic {
            assert!(matches!(a.id, SetupActionId::InstallWsl | SetupActionId::InstallDistro));
        }
    }
    assert!(r.system.is_elevated == Some(false), "Vixalune must not be run as administrator for detection");
}

#[tokio::test]
#[ignore = "probes the real machine; run with --ignored"]
async fn real_wsl_adapter_preflight_diagnoses_the_actual_state() {
    let reg = registry();
    let nikto = reg.get(&ToolId::new("nikto").unwrap()).unwrap().manifest;
    let adapter = WslAptAdapter::new(Arc::new(SystemCommandExecutor), Arc::new(SystemExecutableLocator), None);
    let (sink, cancel) = (Print, CancellationToken::new());
    let ctx = AdapterContext { tool: &nikto, cancel: &cancel, sink: &sink };
    let report = detector().detect().await;
    match adapter.preflight(&ctx, &nikto.install[0]).await {
        Ok(p) => {
            println!("preflight ok: {p:?}");
            assert!(report.wsl.usable_distros().next().is_some(), "preflight succeeded so a WSL 2 distribution must exist");
        }
        Err(AppError::Platform(f)) => {
            println!("diagnosed: {:?} ({:?}) -> {}", f.kind, f.layer, f.detail);
            assert!(matches!(f.kind, FailureKind::WslUnavailable | FailureKind::DistributionMissing | FailureKind::VirtualizationDisabled), "{f:?}");
            assert!(f.recovery().len() > 30);
            if report.wsl.installed == Some(false) {
                assert_eq!(f.kind, FailureKind::WslUnavailable);
            }
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "probes the real machine; run with --ignored"]
async fn real_container_adapter_preflight_diagnoses_the_actual_state() {
    let reg = registry();
    let semgrep = reg.get(&ToolId::new("semgrep").unwrap()).unwrap().manifest;
    let regs: Vec<String> = DEFAULT_APPROVED_REGISTRIES.iter().map(|s| s.to_string()).collect();
    let adapter = ContainerAdapter::new(Arc::new(SystemCommandExecutor), Arc::new(SystemExecutableLocator), regs, std::env::var_os("USERPROFILE").map(PathBuf::from));
    let (sink, cancel) = (Print, CancellationToken::new());
    let ctx = AdapterContext { tool: &semgrep, cancel: &cancel, sink: &sink };
    let report = detector().detect().await;
    match adapter.preflight(&ctx, &semgrep.install[0]).await {
        Ok(p) => {
            println!("preflight ok: {p:?}");
            assert_eq!(report.readiness.containers.state, Readiness::Ready);
        }
        Err(AppError::Platform(f)) => {
            println!("diagnosed: {:?} ({:?}) -> {}", f.kind, f.layer, f.detail);
            assert!(matches!(f.kind, FailureKind::ContainerRuntimeStopped | FailureKind::RemoteContainerContext | FailureKind::UnsupportedContainerOs), "{f:?}");
            assert_ne!(report.readiness.containers.state, Readiness::Ready);
        }
        Err(AppError::PrerequisiteMissing(m)) => {
            println!("no container CLI: {m}");
            assert!(report.containers.runtime.is_none());
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}
