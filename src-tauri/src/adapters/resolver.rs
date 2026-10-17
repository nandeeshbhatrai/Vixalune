//! Basic `DependencyResolver`: picks the first install method whose prerequisites are
//! satisfiable and reports whatever is still missing.

use crate::domain::environment::EnvironmentReport;
use crate::domain::manifest::{InstallSpec, Prerequisite, RuntimeKind, ToolManifest};
use crate::domain::process::InstallPlan;
use crate::domain::{AppError, AppResult};
use crate::ports::DependencyResolver;

#[derive(Default)]
pub struct BasicDependencyResolver;

fn needed_for(spec: &InstallSpec) -> Vec<Prerequisite> {
    match spec {
        InstallSpec::Winget { .. } => vec![Prerequisite::Winget],
        InstallSpec::WslApt { .. } => vec![Prerequisite::Wsl2],
        InstallSpec::ContainerImage { .. } => vec![Prerequisite::Docker],
        InstallSpec::DownloadArchive { .. } => vec![],
    }
}

impl DependencyResolver for BasicDependencyResolver {
    fn resolve(&self, manifest: &ToolManifest, env: &EnvironmentReport) -> AppResult<InstallPlan> {
        let missing_for = |spec: &InstallSpec| -> Vec<Prerequisite> {
            let mut all = needed_for(spec);
            for p in &manifest.prerequisites {
                if !all.contains(p) {
                    all.push(*p);
                }
            }
            all.into_iter().filter(|p| !env.has(*p)).collect()
        };

        // Prefer a method that is ready now; otherwise the one with the fewest gaps.
        let best = manifest
            .install
            .iter()
            .filter(|s| manifest.platforms.contains(&s.runtime()) || s.runtime() == RuntimeKind::WindowsNative)
            .map(|s| (missing_for(s), s))
            .min_by_key(|(missing, _)| missing.len())
            .ok_or_else(|| AppError::Validation(format!("tool '{}' has no usable install method", manifest.id)))?;

        Ok(InstallPlan {
            tool_id: manifest.id.clone(),
            method: best.1.clone(),
            missing_prerequisites: best.0,
            requires_elevation: manifest.requires_elevation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::environment::{ComponentState, ComponentStatus};

    fn env(winget: bool) -> EnvironmentReport {
        let state = if winget { ComponentState::Available { version: None } } else { ComponentState::Missing };
        EnvironmentReport {
            os: "windows".into(),
            supported_os: true,
            components: vec![ComponentStatus { component: Prerequisite::Winget, state }],
            issues: vec![],
        }
    }

    fn manifest() -> ToolManifest {
        ToolManifest::from_toml(
            r#"schema_version = 1
id = "t"
name = "T"
summary = "s"
description = "d"
category = "utilities"
license = "MIT"
homepage = "https://example.com"
documentation = "https://example.com"
platforms = ["windows_native"]
prerequisites = ["winget"]
[[install]]
method = "winget"
package_id = "A.B"
[verify]
runtime = "windows_native"
executable = "t"
"#,
        )
        .unwrap()
    }

    #[test]
    fn ready_when_prerequisites_present() {
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(true)).unwrap();
        assert!(plan.is_ready());
    }

    #[test]
    fn reports_missing_prerequisites() {
        let plan = BasicDependencyResolver.resolve(&manifest(), &env(false)).unwrap();
        assert_eq!(plan.missing_prerequisites, vec![Prerequisite::Winget]);
    }
}
