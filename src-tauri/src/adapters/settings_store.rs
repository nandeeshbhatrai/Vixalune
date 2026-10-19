//! User settings (lab allow-list, retention) kept in one small JSON file in the app data folder.
//! The file holds no secrets. A damaged file never blocks start-up: defaults are used and the problem is logged.

use crate::domain::settings::UserSettings;
use crate::domain::{AppError, AppResult};
use crate::ports::SettingsProvider;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct FileSettings {
    path: Option<PathBuf>,
    current: Mutex<UserSettings>,
}

impl FileSettings {
    pub fn open(path: PathBuf) -> Self {
        let loaded = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<UserSettings>(&s).map_err(|e| e.to_string()).and_then(|u| u.validate().map(|_| u).map_err(|e| e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(UserSettings::default()),
            Err(e) => Err(e.to_string()),
        };
        let current = loaded.unwrap_or_else(|e| {
            tracing::warn!("settings file ignored, defaults used: {e}");
            UserSettings::default()
        });
        Self { path: Some(path), current: Mutex::new(current) }
    }

    pub fn in_memory() -> Self {
        Self { path: None, current: Mutex::new(UserSettings::default()) }
    }
}

impl SettingsProvider for FileSettings {
    fn get(&self) -> UserSettings {
        self.current.lock().unwrap().clone()
    }

    fn update(&self, settings: UserSettings) -> AppResult<()> {
        settings.validate()?;
        if let Some(path) = &self.path {
            let json = serde_json::to_string_pretty(&settings).map_err(|e| AppError::Io(e.to_string()))?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // Write to a sibling file first so a crash cannot leave half a settings file behind.
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, json)?;
            std::fs::rename(&tmp, path)?;
        }
        *self.current.lock().unwrap() = settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_then_saved_values_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let s = FileSettings::open(path.clone());
        assert_eq!(s.get(), UserSettings::default());
        let mut next = UserSettings::default();
        next.scope.lab_targets = vec!["192.168.56.0/24".into(), "lab.local".into()];
        next.retention.history_days = 30;
        next.retention.output_days = 7;
        s.update(next.clone()).unwrap();
        assert_eq!(FileSettings::open(path).get(), next);
    }

    #[test]
    fn invalid_updates_are_rejected_and_change_nothing() {
        let s = FileSettings::in_memory();
        let mut bad = UserSettings::default();
        bad.scope.lab_targets = vec!["0.0.0.0/0".into()];
        assert!(s.update(bad).is_err());
        assert_eq!(s.get(), UserSettings::default());
    }

    #[test]
    fn a_damaged_or_invalid_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(FileSettings::open(path.clone()).get(), UserSettings::default());
        std::fs::write(&path, r#"{"scope":{"lab_targets":["0.0.0.0/0"]},"retention":{"history_days":10,"output_days":5}}"#).unwrap();
        assert_eq!(FileSettings::open(path).get(), UserSettings::default(), "a range that wide is not a lab");
    }
}
