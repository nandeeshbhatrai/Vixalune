//! `ToolRegistry` backed by `*.toml` manifests in a directory. Invalid manifests are skipped
//! and reported; one bad file never hides the rest of the catalog.

use crate::domain::manifest::ToolManifest;
use crate::domain::{AppError, AppResult, ToolId};
use crate::ports::{CatalogEntry, ToolQuery, ToolRegistry, TrustLevel};
use std::collections::BTreeMap;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// A manifest larger than this is rejected without being parsed (resource exhaustion).
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
/// Upper bound on manifest files considered, so a directory full of files cannot stall start-up.
pub const MAX_MANIFEST_FILES: usize = 500;

fn read_manifest(path: &Path) -> AppResult<(String, String)> {
    // Regular files only: a symbolic link or junction could point outside the catalog folder.
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(AppError::Validation("manifests must be regular files, not links".into()));
    }
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(AppError::Validation(format!("manifest is larger than {} KiB", MAX_MANIFEST_BYTES / 1024)));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(AppError::Validation("manifest grew while it was being read".into()));
    }
    let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let text = String::from_utf8(bytes).map_err(|_| AppError::Validation("manifest is not valid UTF-8".into()))?;
    Ok((text, digest))
}

pub struct ManifestRegistry {
    entries: std::sync::RwLock<BTreeMap<ToolId, CatalogEntry>>,
    pub rejected: Vec<AppError>,
}

impl ManifestRegistry {
    pub fn load_dir(dir: &Path, trust: TrustLevel) -> AppResult<Self> {
        let mut entries = BTreeMap::new();
        let mut rejected = Vec::new();
        let mut paths: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .take(MAX_MANIFEST_FILES)
            .collect();
        paths.sort();
        for path in paths {
            let load = read_manifest(&path).and_then(|(s, digest)| ToolManifest::from_toml(&s).map(|m| (m, digest)));
            match load {
                Ok((m, digest)) => {
                    if entries.contains_key(&m.id) {
                        rejected.push(AppError::ManifestLoad {
                            path: path.display().to_string(),
                            message: format!("duplicate tool id '{}'", m.id),
                        });
                    } else {
                        entries.insert(m.id.clone(), CatalogEntry { manifest: m, trust, digest });
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
        Ok(Self { entries: std::sync::RwLock::new(entries), rejected })
    }
}

impl ManifestRegistry {
    /// Like `load_dir`, but an unreadable catalog folder yields an empty catalog plus the reason instead of an error:
    /// the application must still start and explain the problem.
    pub fn load_dir_or_empty(dir: &Path, trust: TrustLevel) -> Self {
        Self::load_dir(dir, trust).unwrap_or_else(|e| Self {
            entries: std::sync::RwLock::new(BTreeMap::new()),
            rejected: vec![AppError::ManifestLoad { path: dir.display().to_string(), message: format!("the catalog folder could not be read: {e}") }],
        })
    }
}

impl ManifestRegistry {
    /// Add an entry while the application runs (a community entry the person chose to add). Never replaces an existing entry.
    pub fn add_entry(&self, manifest: ToolManifest, trust: TrustLevel, digest: String) -> AppResult<CatalogEntry> {
        let mut map = self.entries.write().unwrap();
        if map.contains_key(&manifest.id) {
            return Err(AppError::Validation(format!("'{}' is already in the catalog", manifest.id)));
        }
        let entry = CatalogEntry { manifest, trust, digest };
        map.insert(entry.manifest.id.clone(), entry.clone());
        Ok(entry)
    }

    /// Remove an entry that was added at run time. Reviewed (bundled) entries can never be removed this way.
    pub fn remove_entry(&self, id: &ToolId) -> AppResult<()> {
        let mut map = self.entries.write().unwrap();
        match map.get(id) {
            None => Err(AppError::NotFound(format!("tool '{id}'"))),
            Some(e) if e.trust == TrustLevel::Bundled => Err(AppError::Validation("reviewed catalog entries cannot be removed".into())),
            Some(_) => {
                map.remove(id);
                Ok(())
            }
        }
    }
}

impl ToolRegistry for ManifestRegistry {
    fn list(&self, query: &ToolQuery) -> Vec<CatalogEntry> {
        let needle = query.text.as_deref().map(str::to_lowercase).filter(|s| !s.is_empty());
        self.entries
            .read()
            .unwrap()
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
        self.entries.read().unwrap().get(id).cloned().ok_or_else(|| AppError::NotFound(format!("tool '{id}'")))
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
        assert_eq!(reg.list(&ToolQuery::default()).len(), 13);
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

