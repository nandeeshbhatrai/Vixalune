//! `ToolRegistry` backed by `*.toml` manifests in a directory. Invalid manifests are skipped
//! and reported; one bad file never hides the rest of the catalog.

use crate::domain::manifest::ToolManifest;
use crate::domain::{AppError, AppResult, ToolId};
use crate::ports::{CatalogEntry, ToolQuery, ToolRegistry, TrustLevel};
use std::collections::BTreeMap;
use std::path::Path;

pub struct ManifestRegistry {
    entries: BTreeMap<ToolId, CatalogEntry>,
    pub rejected: Vec<AppError>,
}

impl ManifestRegistry {
    pub fn load_dir(dir: &Path, trust: TrustLevel) -> AppResult<Self> {
        let mut entries = BTreeMap::new();
        let mut rejected = Vec::new();
        let mut paths: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.is_file())
            .collect();
        paths.sort();
        for path in paths {
            let load = std::fs::read_to_string(&path)
                .map_err(AppError::from)
                .and_then(|s| ToolManifest::from_toml(&s));
            match load {
                Ok(m) => {
                    if entries.contains_key(&m.id) {
                        rejected.push(AppError::ManifestLoad {
                            path: path.display().to_string(),
                            message: format!("duplicate tool id '{}'", m.id),
                        });
                    } else {
                        entries.insert(m.id.clone(), CatalogEntry { manifest: m, trust });
                    }
                }
                Err(e) => rejected.push(AppError::ManifestLoad { path: path.display().to_string(), message: e.to_string() }),
            }
        }
        // Dependencies must name other catalog tools.
        let ids: std::collections::HashSet<ToolId> = entries.keys().cloned().collect();
        let broken: Vec<ToolId> = entries
            .values()
            .filter(|e| e.manifest.dependencies.iter().any(|d| !ids.contains(d)))
            .map(|e| e.manifest.id.clone())
            .collect();
        for id in broken {
            entries.remove(&id);
            rejected.push(AppError::ManifestLoad { path: id.to_string(), message: "depends on a tool that is not in the catalog".into() });
        }
        Ok(Self { entries, rejected })
    }
}

impl ToolRegistry for ManifestRegistry {
    fn list(&self, query: &ToolQuery) -> Vec<CatalogEntry> {
        let needle = query.text.as_deref().map(str::to_lowercase).filter(|s| !s.is_empty());
        self.entries
            .values()
            .filter(|e| query.category.is_none_or(|c| c == e.manifest.category))
            .filter(|e| match &needle {
                None => true,
                Some(n) => {
                    let m = &e.manifest;
                    m.name.to_lowercase().contains(n)
                        || m.summary.to_lowercase().contains(n)
                        || m.id.as_str().contains(n.as_str())
                }
            })
            .cloned()
            .collect()
    }

    fn get(&self, id: &ToolId) -> AppResult<CatalogEntry> {
        self.entries.get(id).cloned().ok_or_else(|| AppError::NotFound(format!("tool '{id}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::manifest::Category;

    fn manifest(id: &str, category: &str) -> String {
        crate::domain::manifest::fixtures::VALID
            .replace("id = \"nmap\"", &format!("id = \"{id}\""))
            .replace("category = \"reconnaissance\"", &format!("category = \"{category}\""))
    }

    #[test]
    fn bundled_manifests_are_all_valid() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("manifests");
        let reg = ManifestRegistry::load_dir(&dir, TrustLevel::Bundled).unwrap();
        assert!(reg.rejected.is_empty(), "{:?}", reg.rejected);
        assert_eq!(reg.list(&ToolQuery::default()).len(), 5);
    }

    #[test]
    fn loads_valid_skips_invalid_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.toml"), manifest("alpha", "network")).unwrap();
        std::fs::write(dir.path().join("b.toml"), manifest("beta", "passwords")).unwrap();
        std::fs::write(dir.path().join("bad.toml"), "schema_version = 1\nid = \"../x\"").unwrap();
        std::fs::write(dir.path().join("note.txt"), "ignored").unwrap();
        let orphan = manifest("orphan", "network").replace("platforms = [\"windows_native\"]", "platforms = [\"windows_native\"]\ndependencies = [\"ghost\"]");
        std::fs::write(dir.path().join("orphan.toml"), orphan).unwrap();

        let reg = ManifestRegistry::load_dir(dir.path(), TrustLevel::Bundled).unwrap();
        assert_eq!(reg.rejected.len(), 2); // invalid id + dependency on a missing tool
        assert_eq!(reg.list(&ToolQuery::default()).len(), 2);
        let q = ToolQuery { text: None, category: Some(Category::Passwords) };
        assert_eq!(reg.list(&q).len(), 1);
        let q = ToolQuery { text: Some("ALP".into()), category: None };
        assert_eq!(reg.list(&q)[0].manifest.id.as_str(), "alpha");
        assert!(reg.get(&ToolId::new("missing").unwrap()).is_err());
    }
}
