//! Application configuration (TOML, stored in the app data directory). Contains paths and
//! resource limits only - never credentials.

use crate::domain::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub tools_dir: PathBuf,
    pub labs_dir: PathBuf,
    pub database_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub wsl_distribution: Option<String>,
    pub docker_executable: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    pub max_concurrent_installs: u8,
    pub max_concurrent_processes: u8,
    pub max_output_buffer_mb: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub storage: StorageConfig,
    pub runtime: RuntimeConfig,
    pub resources: ResourceConfig,
    /// Remote manifest sources are untrusted and disabled until a later phase.
    pub remote_manifests_enabled: bool,
}

impl AppConfig {
    pub fn defaults_for(data_dir: &Path) -> Self {
        Self {
            storage: StorageConfig {
                tools_dir: data_dir.join("tools"),
                labs_dir: data_dir.join("labs"),
                database_path: data_dir.join("vixalune.db"),
            },
            runtime: RuntimeConfig { wsl_distribution: None, docker_executable: None },
            resources: ResourceConfig { max_concurrent_installs: 1, max_concurrent_processes: 4, max_output_buffer_mb: 16 },
            remote_manifests_enabled: false,
        }
    }

    pub fn validate(&self) -> AppResult<()> {
        let r = &self.resources;
        if !(1..=8).contains(&r.max_concurrent_installs) || !(1..=32).contains(&r.max_concurrent_processes) || !(1..=512).contains(&r.max_output_buffer_mb) {
            return Err(AppError::Validation("resource limits are out of range".into()));
        }
        if let Some(d) = &self.runtime.wsl_distribution {
            if d.is_empty() || !d.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
                return Err(AppError::Validation("wsl_distribution contains invalid characters".into()));
            }
        }
        Ok(())
    }

    /// Load from `path`; a missing file yields defaults. Does not write.
    pub fn load_or_default(path: &Path, data_dir: &Path) -> AppResult<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let c: Self = toml::from_str(&s).map_err(|e| AppError::Validation(format!("config.toml: {e}")))?;
                c.validate()?;
                Ok(c)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::defaults_for(data_dir)),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        self.validate()?;
        let s = toml::to_string_pretty(self).map_err(|e| AppError::Io(e.to_string()))?;
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, s)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let loaded = AppConfig::load_or_default(&path, dir.path()).unwrap();
        assert!(!loaded.remote_manifests_enabled);
        loaded.save(&path).unwrap();
        assert_eq!(AppConfig::load_or_default(&path, dir.path()).unwrap(), loaded);
    }

    #[test]
    fn rejects_out_of_range_limits() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = AppConfig::defaults_for(dir.path());
        c.resources.max_concurrent_installs = 0;
        assert!(c.validate().is_err());
    }
}
