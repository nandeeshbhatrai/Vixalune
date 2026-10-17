//! `DependencyResolver`: picks an install method that has a working adapter and describes, before
//! anything runs, what the install needs (prerequisites, size, administrator rights, restart).

use crate::domain::environment::{ComponentState, EnvironmentReport};
use crate::domain::manifest::{Architecture, ElevationNeed, InstallSpec, Prerequisite, PrerequisiteSpec, ToolManifest};
use crate::domain::process::{InstallPlan, PrerequisiteStatus};
use crate::domain::AppResult;
use crate::ports::DependencyResolver;

#[derive(Default)]
pub struct BasicDependencyResolver;

fn implied(spec: &InstallSpec) -> Option<(Prerequisite, &'static str)> {
    match spec {
        InstallSpec::Winget { .. } => Some((Prerequisite::Winget, "Installs the tool from the Windows Package Manager.")),
        InstallSpec::WslApt { .. } => Some((Prerequisite::Wsl2, "Installs the tool inside a WSL 2 Linux distribution.")),
        InstallSpec::ContainerImage { .. } => Some((Prerequisite::Docker, "Runs the tool as a container image.")),
        InstallSpec::DownloadArchive { .. } => None,
    }
}

fn satisfied(env: &EnvironmentReport, kind: Prerequisite) -> Option<bool> {
    env.components.iter().find(|c| c.component == kind).and_then(|c| match c.state {
        ComponentState::Available { .. } => Some(true),
        ComponentState::Missing => Some(false),
        ComponentState::Unknown { .. } => None,
    })
}

fn rank(e: ElevationNeed) -> u8 {
    match e {
        ElevationNeed::None => 0,
        ElevationNeed::Possible => 1,
        ElevationNeed::Required => 2,
    }
}

fn status_for(manifest: &ToolManifest, env: &EnvironmentReport, spec: &InstallSpec) -> Vec<PrerequisiteStatus> {
    let mut specs: Vec<PrerequisiteSpec> = manifest.prerequisites.clone();
    if let Some((kind, reason)) = implied(spec) {
        if !specs.iter().any(|p| p.kind == kind) {
            specs.push(PrerequisiteSpec { kind, reason: reason.into(), approx_size_mb: None, requires_admin: false, requires_restart: false });
        }
    }
    specs.into_iter().map(|s| PrerequisiteStatus { satisfied: satisfied(env, s.kind), spec: s }).collect()
}

fn unsupported_reason(manifest: &ToolManifest, env: &EnvironmentReport) -> Option<String> {
    if !env.supported_os {
        return Some("Vixalune installs tools on Windows only.".into());
    }
    if let Some(build) = env.windows_build {
        if build < manifest.windows.min_build {
            return Some(format!("{} needs Windows build {} or newer; this PC has build {build}.", manifest.name, manifest.windows.min_build));
        }
    }
    let arch = match env.architecture.as_str() {
        "x64" => Some(Architecture::X64),
        "arm64" => Some(Architecture::Arm64),
        _ => None,
    };
    if let Some(a) = arch {
        if !manifest.windows.architectures.contains(&a) {
            return Some(format!("{} does not support this PC's processor architecture ({}).", manifest.name, env.architecture));
        }
    }
    None
}

impl DependencyResolver for BasicDependencyResolver {
    fn resolve(&self, manifest: &ToolManifest, env: &EnvironmentReport, usable: &dyn Fn(&InstallSpec) -> bool) -> AppResult<InstallPlan> {
        let missing_count = |spec: &InstallSpec| status_for(manifest, env, spec).iter().filter(|s| s.satisfied != Some(true)).count();

        let chosen = manifest.install.iter().filter(|s| usable(s)).min_by_key(|s| missing_count(s));
        let (spec, mut reason) = match chosen {
            Some(s) => (s.clone(), None),
            None => (
                manifest.install[0].clone(),
                Some(format!(
                    "This version of Vixalune cannot install {} yet (needs the {:?} installer).",
                    manifest.name,
                    manifest.install[0].method()
                )),
            ),
        };
        reason = reason.or_else(|| unsupported_reason(manifest, env));

        let prerequisites = status_for(manifest, env, &spec);
        let missing_prerequisites = prerequisites.iter().filter(|p| p.satisfied != Some(true)).map(|p| p.spec.kind).collect();
        let prereq_admin = prerequisites.iter().any(|p| p.spec.requires_admin);
        let elevation = if prereq_admin && rank(manifest.privileges.install) < rank(ElevationNeed::Required) {
            ElevationNeed::Required
        } else {
            manifest.privileges.install
        };
        Ok(InstallPlan {
            tool_id: manifest.id.clone(),
            method: spec,
            may_require_restart: manifest.privileges.may_require_restart || prerequisites.iter().any(|p| p.spec.requires_restart),
            prerequisites,
            missing_prerequisites,
            dependencies: manifest.dependencies.clone(),
            elevation,
            network_required: manifest.network.install,
            approx_download_mb: manifest.storage.approx_download_mb,
            approx_disk_mb: manifest.storage.approx_disk_mb,
            license: manifest.license.clone(),
            integrity: manifest.integrity.level,
            unsupported_reason: reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::environment::ComponentStatus;
    use crate::domain::manifest::fixtures::VALID;

    fn env(winget: Option<bool>, build: Option<u32>, arch: &str) -> EnvironmentReport {
        let state = match winget {
            Some(true) => ComponentState::Available { version: None },
            Some(false) => ComponentState::Missing,
            None => ComponentState::Unknown { reason: "timed out".into() },
        };
        EnvironmentReport {
            os: "windows".into(),
            supported_os: true,
            windows_build: build,
            architecture: arch.into(),
            components: vec![ComponentStatus { component: Prerequisite::Winget, state }],
            issues: vec![],
        }
    }

    fn manifest() -> ToolManifest {
        ToolManifest::from_toml(VALID).unwrap()
    }

    fn any(_: &InstallSpec) -> bool {
        true
    }

    #[test]
    fn ready_when_prerequisites_present() {
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(Some(true), Some(26100), "x64"), &any).unwrap();
        assert!(plan.is_ready());
        assert_eq!(plan.elevation, ElevationNeed::Possible);
        assert!(plan.network_required);
        assert_eq!(plan.prerequisites[0].spec.reason, "Installs Nmap from the Windows Package Manager.");
    }

    #[test]
    fn reports_missing_and_unverifiable_prerequisites() {
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(Some(false), Some(26100), "x64"), &any).unwrap();
        assert_eq!(plan.missing_prerequisites, vec![Prerequisite::Winget]);
        assert!(!plan.is_ready());
        // Could not be checked: not assumed present.
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(None, Some(26100), "x64"), &any).unwrap();
        assert_eq!(plan.missing_prerequisites, vec![Prerequisite::Winget]);
        assert_eq!(plan.prerequisites[0].satisfied, None);
    }

    #[test]
    fn rejects_old_windows_and_wrong_architecture() {
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(Some(true), Some(18363), "x64"), &any).unwrap();
        assert!(plan.unsupported_reason.unwrap().contains("19045"));
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(Some(true), Some(26100), "arm64"), &any).unwrap();
        assert!(plan.unsupported_reason.unwrap().contains("architecture"));
        // Unknown build is not treated as too old.
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(Some(true), None, "x64"), &any).unwrap();
        assert!(plan.is_ready());
    }

    #[test]
    fn unusable_install_methods_are_unsupported_not_missing() {
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(Some(true), Some(26100), "x64"), &|_| false).unwrap();
        assert!(plan.unsupported_reason.as_ref().unwrap().contains("cannot install"));
        assert!(!plan.is_ready());
    }

    #[test]
    fn prerequisite_admin_requirement_raises_elevation() {
        let mut m = manifest();
        m.prerequisites[0].requires_admin = true;
        m.prerequisites[0].requires_restart = true;
        let plan = BasicDependencyResolver.resolve(&m, &env(Some(true), Some(26100), "x64"), &any).unwrap();
        assert_eq!(plan.elevation, ElevationNeed::Required);
        assert!(plan.may_require_restart);
    }
}
