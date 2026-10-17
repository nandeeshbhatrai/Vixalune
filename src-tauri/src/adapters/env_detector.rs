//! Windows environment detector. Probes use fixed program names and fixed argument arrays.

use crate::domain::environment::{issues_for, ComponentState, ComponentStatus, EnvironmentReport};
use crate::domain::manifest::Prerequisite;
use crate::ports::EnvironmentDetector;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
enum Probe {
    Ran { success: bool, stdout: String },
    NotFound,
    Failed(String),
}

fn run_probe(program: &str, args: &[&str]) -> Probe {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Probe::NotFound,
        Err(e) => return Probe::Failed(e.to_string()),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut buf = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.by_ref().take(64 * 1024).read_to_end(&mut buf);
                }
                return Probe::Ran { success: status.success(), stdout: String::from_utf8_lossy(&buf).trim().to_string() };
            }
            Ok(None) if start.elapsed() > PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Probe::Failed("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Probe::Failed(e.to_string()),
        }
    }
}

fn to_state(p: Probe, keep_version: bool) -> ComponentState {
    match p {
        Probe::Ran { success: true, stdout } => ComponentState::Available {
            version: keep_version.then(|| stdout.lines().next().unwrap_or("").to_string()).filter(|s| !s.is_empty()),
        },
        Probe::Ran { success: false, .. } | Probe::NotFound => ComponentState::Missing,
        Probe::Failed(reason) => ComponentState::Unknown { reason },
    }
}

#[cfg(windows)]
fn windows_build() -> Option<u32> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok()?
        .get_value::<String, _>("CurrentBuildNumber")
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(not(windows))]
fn windows_build() -> Option<u32> {
    None
}

fn architecture() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "x64".into(),
        "aarch64" => "arm64".into(),
        other => other.into(),
    }
}

#[derive(Default)]
pub struct SystemEnvironmentDetector;

impl EnvironmentDetector for SystemEnvironmentDetector {
    fn detect(&self) -> EnvironmentReport {
        let supported_os = cfg!(windows);
        let components = if supported_os {
            vec![
                ComponentStatus { component: Prerequisite::Winget, state: to_state(run_probe("winget", &["--version"]), true) },
                ComponentStatus { component: Prerequisite::Wsl2, state: to_state(run_probe("wsl.exe", &["--status"]), false) },
                ComponentStatus { component: Prerequisite::Docker, state: to_state(run_probe("docker", &["--version"]), true) },
            ]
        } else {
            [Prerequisite::Winget, Prerequisite::Wsl2, Prerequisite::Docker]
                .into_iter()
                .map(|component| ComponentStatus { component, state: ComponentState::Unknown { reason: "not running on Windows".into() } })
                .collect()
        };
        let issues = issues_for(&components, supported_os);
        EnvironmentReport {
            os: std::env::consts::OS.to_string(),
            supported_os,
            windows_build: windows_build(),
            architecture: architecture(),
            components,
            issues,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_program_is_reported_missing() {
        let p = run_probe("vixalune-definitely-not-a-real-program", &[]);
        assert!(matches!(to_state(p, false), ComponentState::Missing));
    }

    #[test]
    fn detect_returns_all_components() {
        let r = SystemEnvironmentDetector.detect();
        assert_eq!(r.components.len(), 3);
        #[cfg(windows)]
        assert!(r.windows_build.is_some_and(|b| b >= 19041), "{:?}", r.windows_build);
        assert!(matches!(r.architecture.as_str(), "x64" | "arm64"));
    }
}
