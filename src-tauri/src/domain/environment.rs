//! Environment readiness model, produced by an `EnvironmentDetector`.

use super::manifest::Prerequisite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ComponentState {
    Available { version: Option<String> },
    Missing,
    /// The probe could not decide (timeout, unsupported OS, ...).
    Unknown { reason: String },
}

impl ComponentState {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub component: Prerequisite,
    #[serde(flatten)]
    pub state: ComponentState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupIssue {
    pub severity: Severity,
    pub title: String,
    pub recovery: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentReport {
    pub os: String,
    pub supported_os: bool,
    /// Windows build number (e.g. 26100), when it could be read.
    pub windows_build: Option<u32>,
    /// `x64` or `arm64` of the running system.
    pub architecture: String,
    /// Free bytes on the volume that holds tools and downloads, when known.
    #[serde(default)]
    pub free_disk_bytes: Option<u64>,
    pub components: Vec<ComponentStatus>,
    pub issues: Vec<SetupIssue>,
}

impl EnvironmentReport {
    pub fn has(&self, c: Prerequisite) -> bool {
        self.components.iter().any(|s| s.component == c && s.state.is_available())
    }
}

/// Turn component states into user-facing setup issues.
pub fn issues_for(components: &[ComponentStatus], supported_os: bool) -> Vec<SetupIssue> {
    let mut out = Vec::new();
    if !supported_os {
        out.push(SetupIssue {
            severity: Severity::Blocking,
            title: "Unsupported operating system".into(),
            recovery: "Vixalune supports Windows 10 (22H2) and Windows 11 only.".into(),
        });
    }
    for c in components {
        if c.state.is_available() {
            continue;
        }
        let (name, recovery, sev) = match c.component {
            Prerequisite::Winget => (
                "winget (App Installer)",
                "Install or update 'App Installer' from the Microsoft Store, then re-check.",
                Severity::Warning,
            ),
            Prerequisite::Wsl2 => (
                "WSL 2",
                "Open a terminal as administrator, run 'wsl --install', restart Windows, then re-check. Only needed for Linux-based tools.",
                Severity::Info,
            ),
            Prerequisite::Docker => (
                "Docker",
                "Install Docker Desktop (or Docker Engine inside WSL) and start it, then re-check. Only needed for container labs and tools.",
                Severity::Info,
            ),
        };
        let detail = match &c.state {
            ComponentState::Unknown { reason } => format!("{name} could not be checked: {reason}"),
            _ => format!("{name} was not found"),
        };
        out.push(SetupIssue { severity: sev, title: detail, recovery: recovery.into() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_components_yield_issues_with_recovery() {
        let comps = vec![
            ComponentStatus { component: Prerequisite::Winget, state: ComponentState::Available { version: Some("1.0".into()) } },
            ComponentStatus { component: Prerequisite::Wsl2, state: ComponentState::Missing },
        ];
        let issues = issues_for(&comps, true);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].recovery.contains("wsl --install"));
    }

    #[test]
    fn component_status_serializes_flat() {
        let c = ComponentStatus { component: Prerequisite::Docker, state: ComponentState::Missing };
        let v = serde_json::to_value(c).unwrap();
        assert_eq!(v["component"], "docker");
        assert_eq!(v["state"], "missing");
    }
}

