//! Bundled tutorials. Each one is validated against its tool's manifest when loaded; one that does not validate is
//! rejected (and reported) rather than shown.

use crate::domain::learning::Tutorial;
use crate::domain::{AppError, AppResult, ToolId};
use crate::ports::{LearningCatalog, ToolRegistry};
use std::collections::BTreeMap;

/// `(file name, content)` of the tutorials shipped with this build.
pub const BUNDLED: &[(&str, &str)] = &[
    ("nmap.toml", include_str!("../../../learning/nmap.toml")),
    ("zap.toml", include_str!("../../../learning/zap.toml")),
    ("semgrep.toml", include_str!("../../../learning/semgrep.toml")),
    ("trivy.toml", include_str!("../../../learning/trivy.toml")),
];

pub struct LearningRegistry {
    tutorials: BTreeMap<String, Tutorial>,
    pub rejected: Vec<String>,
}

impl LearningRegistry {
    pub fn load(sources: &[(&str, &str)], registry: &dyn ToolRegistry) -> Self {
        let mut tutorials = BTreeMap::new();
        let mut rejected = Vec::new();
        for (name, content) in sources {
            let parsed = toml::from_str::<Tutorial>(content)
                .map_err(|e| AppError::Validation(format!("{name}: {e}")))
                .and_then(|t| {
                    let m = registry.get(&t.tool_id).map_err(|_| AppError::Validation(format!("{name}: no tool '{}' in the catalog", t.tool_id)))?.manifest;
                    t.validate(&m).map_err(|e| AppError::Validation(format!("{name}: {e}")))?;
                    Ok(t)
                });
            match parsed {
                Ok(t) => {
                    tutorials.insert(t.tool_id.to_string(), t);
                }
                Err(e) => rejected.push(e.to_string()),
            }
        }
        Self { tutorials, rejected }
    }

    pub fn bundled(registry: &dyn ToolRegistry) -> Self {
        Self::load(BUNDLED, registry)
    }
}

impl LearningCatalog for LearningRegistry {
    fn list(&self) -> Vec<Tutorial> {
        self.tutorials.values().cloned().collect()
    }

    fn get(&self, id: &ToolId) -> AppResult<Tutorial> {
        self.tutorials.get(id.as_str()).cloned().ok_or_else(|| AppError::NotFound(format!("a tutorial for '{id}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::registry::ManifestRegistry;
    use crate::ports::TrustLevel;
    use std::path::PathBuf;

    fn manifests() -> ManifestRegistry {
        ManifestRegistry::load_dir(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests"), TrustLevel::Bundled).unwrap()
    }

    #[test]
    fn every_bundled_tutorial_is_valid_and_complete() {
        let r = LearningRegistry::bundled(&manifests());
        assert!(r.rejected.is_empty(), "{:?}", r.rejected);
        let ids: Vec<String> = r.list().iter().map(|t| t.tool_id.to_string()).collect();
        assert_eq!(ids, ["nmap", "semgrep", "trivy", "zap"]);
        for t in r.list() {
            assert!(!t.authorization_warning.is_empty());
            assert!(t.first_run.len() >= 3 && !t.common_errors.is_empty() && !t.links.is_empty());
            assert!(t.links.iter().all(|l| l.url.starts_with("https://")));
        }
    }

    #[test]
    fn exercises_use_the_right_lab_for_the_tool() {
        let r = LearningRegistry::bundled(&manifests());
        for (tool, lab) in [("nmap", "practice-web"), ("zap", "practice-web"), ("semgrep", "sample-code"), ("trivy", "sample-code")] {
            assert_eq!(r.get(&ToolId::new(tool).unwrap()).unwrap().exercise.lab, lab, "{tool}");
        }
        assert!(matches!(r.get(&ToolId::new("wireshark").unwrap()), Err(AppError::NotFound(_))));
    }

    #[test]
    fn a_tutorial_that_disagrees_with_its_manifest_is_rejected() {
        let nmap = BUNDLED[0].1;
        let cases = [
            ("unknown profile", nmap.replace("profile = \"service-scan\"", "profile = \"nope\"")),
            ("unknown parameter", nmap.replace("name = \"ports\"", "name = \"nope\"")),
            ("foreign link", nmap.replace("https://nmap.org\"", "https://example.org\"")),
            ("insecure link", nmap.replace("https://nmap.org/book/man.html", "http://nmap.org/book/man.html")),
            ("lookalike link", nmap.replace("https://nmap.org\"", "https://nmap.org.evil.example\"")),
            ("unknown lab", nmap.replace("lab = \"practice-web\"", "lab = \"production\"")),
            ("no warning", nmap.replace("Only scan computers and networks you own or have written permission to test.", "Be careful.").replace("not legal permission", "not a legal shield")),
            ("other tool", nmap.replace("tool_id = \"nmap\"", "tool_id = \"nikto\"")),
        ];
        for (what, text) in cases {
            let r = LearningRegistry::load(&[("x.toml", text.as_str())], &manifests());
            assert_eq!(r.list().len(), 0, "{what} should be rejected");
            assert_eq!(r.rejected.len(), 1, "{what}");
        }
        let r = LearningRegistry::load(&[("bad.toml", "this is not toml [")], &manifests());
        assert_eq!(r.rejected.len(), 1);
    }
}
