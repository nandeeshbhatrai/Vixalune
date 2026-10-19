//! User-editable settings that affect safety and privacy: the lab allow-list and data retention.

use super::errors::{AppError, AppResult};
use super::scope::ScopePolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeSettings {
    /// Host names, IP addresses or CIDR ranges of labs you manage. Loopback is always a lab.
    #[serde(default)]
    pub lab_targets: Vec<String>,
}

/// How long things are kept. Always explicit: nothing is kept "forever" by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionSettings {
    /// Run history records (command, times, exit status, scope) are deleted after this many days.
    pub history_days: u32,
    /// Stored tool output is deleted after this many days (0 = do not keep output once the run ends).
    pub output_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSettings {
    pub scope: ScopeSettings,
    pub retention: RetentionSettings,
}

pub const DEFAULT_HISTORY_DAYS: u32 = 90;
pub const DEFAULT_OUTPUT_DAYS: u32 = 14;

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            scope: ScopeSettings { lab_targets: vec![] },
            retention: RetentionSettings { history_days: DEFAULT_HISTORY_DAYS, output_days: DEFAULT_OUTPUT_DAYS },
        }
    }
}

impl UserSettings {
    pub fn validate(&self) -> AppResult<()> {
        ScopePolicy::parse(&self.scope.lab_targets)?;
        let r = &self.retention;
        if !(1..=3650).contains(&r.history_days) {
            return Err(AppError::Validation("history retention must be 1-3650 days".into()));
        }
        if r.output_days > 3650 {
            return Err(AppError::Validation("output retention must be 0-3650 days".into()));
        }
        if r.output_days > r.history_days {
            return Err(AppError::Validation("output cannot be kept longer than the history that refers to it".into()));
        }
        Ok(())
    }

    pub fn policy(&self) -> AppResult<ScopePolicy> {
        ScopePolicy::parse(&self.scope.lab_targets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_explicit_and_valid() {
        let s = UserSettings::default();
        assert!(s.validate().is_ok());
        assert_eq!((s.retention.history_days, s.retention.output_days), (90, 14));
    }

    #[test]
    fn rejects_bad_labs_and_retention() {
        let mut s = UserSettings::default();
        s.scope.lab_targets = vec!["10.0.0.0/8".into()];
        assert!(s.validate().is_err());
        s.scope.lab_targets = vec!["192.168.56.0/24".into()];
        assert!(s.validate().is_ok());
        s.retention.history_days = 0;
        assert!(s.validate().is_err());
        s.retention = RetentionSettings { history_days: 30, output_days: 60 };
        assert!(s.validate().is_err());
        s.retention = RetentionSettings { history_days: 30, output_days: 0 };
        assert!(s.validate().is_ok());
    }
}
