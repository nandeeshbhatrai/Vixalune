//! Explicit installation lifecycle. All state changes go through [`InstallState::apply`].

use super::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    NotInstalled,
    Queued,
    Downloading,
    Installing,
    Verifying,
    Installed,
    UpdateAvailable,
    Uninstalling,
    Cancelling,
    Cancelled,
    /// An install/uninstall step failed.
    Failed,
    /// The tool is present but its verification check did not pass.
    VerificationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallEvent {
    Enqueue,
    BeginDownload,
    BeginInstall,
    InstallFinished,
    VerifyRequested,
    VerifyPassed,
    VerifyFailed,
    Fail,
    RequestCancel,
    CancelConfirmed,
    Retry,
    UpdateDetected,
    BeginUninstall,
    UninstallFinished,
}

impl InstallState {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Downloading | Self::Installing | Self::Verifying | Self::Uninstalling | Self::Cancelling
        )
    }

    pub fn apply(self, event: InstallEvent) -> AppResult<Self> {
        use InstallEvent as E;
        use InstallState as S;
        let next = match (self, event) {
            (S::NotInstalled | S::UpdateAvailable, E::Enqueue) => S::Queued,
            (S::Failed | S::Cancelled | S::VerificationFailed, E::Retry) => S::Queued,
            (S::Queued, E::BeginDownload) => S::Downloading,
            (S::Queued, E::BeginInstall) => S::Installing,
            (S::Downloading, E::BeginInstall) => S::Installing,
            (S::Installing, E::InstallFinished) => S::Verifying,
            (S::Installed | S::UpdateAvailable | S::VerificationFailed, E::VerifyRequested) => S::Verifying,
            (S::Verifying, E::VerifyPassed) => S::Installed,
            (S::Verifying, E::VerifyFailed) => S::VerificationFailed,
            (S::Queued | S::Downloading | S::Installing | S::Verifying | S::Uninstalling, E::Fail) => S::Failed,
            (S::Queued | S::Downloading | S::Installing, E::RequestCancel) => S::Cancelling,
            (S::Cancelling, E::CancelConfirmed) => S::Cancelled,
            (S::Installed, E::UpdateDetected) => S::UpdateAvailable,
            (S::Installed | S::UpdateAvailable | S::Failed | S::VerificationFailed, E::BeginUninstall) => S::Uninstalling,
            (S::Uninstalling, E::UninstallFinished) => S::NotInstalled,
            (from, event) => {
                return Err(AppError::InvalidTransition {
                    from: format!("{from:?}"),
                    event: format!("{event:?}"),
                })
            }
        };
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use InstallEvent as E;
    use InstallState as S;

    fn run(start: S, events: &[E]) -> AppResult<S> {
        events.iter().try_fold(start, |s, e| s.apply(*e))
    }

    #[test]
    fn happy_path_install() {
        let end = run(S::NotInstalled, &[E::Enqueue, E::BeginDownload, E::BeginInstall, E::InstallFinished, E::VerifyPassed]);
        assert_eq!(end.unwrap(), S::Installed);
    }

    #[test]
    fn cancel_then_retry() {
        let end = run(S::NotInstalled, &[E::Enqueue, E::BeginInstall, E::RequestCancel, E::CancelConfirmed, E::Retry]);
        assert_eq!(end.unwrap(), S::Queued);
    }

    #[test]
    fn failed_verification_is_distinct_and_retryable() {
        let failed = run(S::Installed, &[E::VerifyRequested, E::VerifyFailed]).unwrap();
        assert_eq!(failed, S::VerificationFailed);
        assert_eq!(failed.apply(E::Retry).unwrap(), S::Queued);
        assert_eq!(failed.apply(E::BeginUninstall).unwrap(), S::Uninstalling);
    }

    #[test]
    fn uninstall_path() {
        let end = run(S::Installed, &[E::BeginUninstall, E::UninstallFinished]).unwrap();
        assert_eq!(end, S::NotInstalled);
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        assert!(S::NotInstalled.apply(E::VerifyPassed).is_err());
        assert!(S::Installed.apply(E::Enqueue).is_err());
        assert!(S::Verifying.apply(E::RequestCancel).is_err());
        assert!(S::Uninstalling.apply(E::RequestCancel).is_err());
    }

    #[test]
    fn busy_states() {
        assert!(S::Installing.is_busy());
        assert!(!S::Installed.is_busy());
    }
}
