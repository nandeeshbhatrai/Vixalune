//! Application error type. Every variant carries a stable `code` and a human-readable
//! recovery instruction so the UI can show actionable errors (requirement 9).

use super::failure::PlatformFailure;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AppError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("manifest at '{path}' could not be loaded: {message}")]
    ManifestLoad { path: String, message: String },
    #[error("{0} was not found")]
    NotFound(String),
    #[error("invalid state transition: cannot apply '{event}' while '{from}'")]
    InvalidTransition { from: String, event: String },
    #[error("environment problem: {0}")]
    Environment(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("administrator privileges are required for: {0}")]
    ElevationRequired(String),
    #[error("operation was cancelled")]
    Cancelled,
    #[error("{0}")]
    Busy(String),
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("a required prerequisite is missing: {0}")]
    PrerequisiteMissing(String),
    #[error("command failed: {message}")]
    CommandFailed { code: Option<i32>, message: String },
    #[error("verification did not pass: {0}")]
    VerificationFailed(String),
    #[error("timed out: {0}")]
    TimedOut(String),
    #[error("{}: {}", .0.title(), .0.detail)]
    Platform(PlatformFailure),
    #[error("'{0}' is not implemented in this build yet")]
    NotImplemented(&'static str),
    #[error("{0}")]
    Ai(super::ai::AiError),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation",
            Self::ManifestLoad { .. } => "manifest_load",
            Self::NotFound(_) => "not_found",
            Self::InvalidTransition { .. } => "invalid_transition",
            Self::Environment(_) => "environment",
            Self::Io(_) => "io",
            Self::Database(_) => "database",
            Self::ElevationRequired(_) => "elevation_required",
            Self::Cancelled => "cancelled",
            Self::NotImplemented(_) => "not_implemented",
            Self::Busy(_) => "busy",
            Self::ApprovalRequired(_) => "approval_required",
            Self::Unsupported(_) => "unsupported",
            Self::PrerequisiteMissing(_) => "prerequisite_missing",
            Self::CommandFailed { .. } => "command_failed",
            Self::VerificationFailed(_) => "verification_failed",
            Self::TimedOut(_) => "timed_out",
            Self::Platform(f) => f.code(),
            Self::Ai(e) => e.kind.code(),
        }
    }

    /// What the user should do next. Never empty.
    pub fn recovery(&self) -> &'static str {
        match self {
            Self::Validation(_) => "Correct the highlighted value and try again.",
            Self::ManifestLoad { .. } => {
                "The tool definition was rejected. Update the catalog, or report the manifest to its maintainer."
            }
            Self::NotFound(_) => "Refresh the catalog and try again.",
            Self::InvalidTransition { .. } => {
                "Wait for the current operation to finish, or cancel it, then retry."
            }
            Self::Environment(_) => "Open Dashboard > Setup issues and follow the fix instructions.",
            Self::Io(_) => "Check that the storage location exists, is writable and has free space (Settings > Storage).",
            Self::Database(_) => "Restart the app. If the problem persists, back up and remove the local database file.",
            Self::ElevationRequired(_) => {
                "Approve the Windows administrator prompt when asked, or choose an install method that does not need it."
            }
            Self::Cancelled => "Nothing was changed. Start the operation again when ready.",
            Self::NotImplemented(_) => "This feature is planned for a later phase.",
            Self::Busy(_) => "Wait for the running operation to finish, or cancel it from Running Jobs, then try again.",
            Self::ApprovalRequired(_) => "Nothing was done. Review the request in the application and approve it there if you want it to go ahead.",
            Self::Unsupported(_) => "This tool cannot be installed on this computer or with this version of Vixalune.",
            Self::PrerequisiteMissing(_) => "Install the missing prerequisite (see Dashboard > Setup issues), then retry.",
            Self::CommandFailed { .. } => "Open the job output for details, fix the cause, then retry.",
            Self::VerificationFailed(_) => {
                "The tool did not pass its check. Use Verify to re-check, Retry to reinstall, or Uninstall to remove it."
            }
            Self::TimedOut(_) => "Check your network connection and available disk space, then retry.",
            Self::Platform(f) => f.recovery(),
            Self::Ai(e) => e.kind.recovery(),
        }
    }
}

impl From<super::ai::AiError> for AppError {
    fn from(e: super::ai::AiError) -> Self {
        Self::Ai(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

/// Wire format sent to the frontend: `{ code, message, recovery }`.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("AppError", 3)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        st.serialize_field("recovery", self.recovery())?;
        st.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_code_and_recovery() {
        let v = serde_json::to_value(AppError::Cancelled).unwrap();
        assert_eq!(v["code"], "cancelled");
        assert!(!v["recovery"].as_str().unwrap().is_empty());
    }
}

