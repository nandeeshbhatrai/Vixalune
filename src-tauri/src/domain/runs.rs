//! Run history records and how a finished process is classified.
//!
//! A *failed* run (the tool crashed, timed out, was refused or exited with an unexpected code) is not the
//! same as a *successful run with findings* (the tool worked and reported something). Learners must be able to
//! tell the two apart at a glance, so they are separate outcomes.

use super::manifest::{ExitCodes, NetworkActivity, OutputFormat};
use super::{AppResult, OperationId, ToolId};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// The tool finished normally and reported nothing of concern.
    Clean,
    /// The tool finished normally and reported findings (open ports, vulnerabilities, ...). Not an error.
    Findings,
    /// The tool failed: unexpected exit code, could not start, or crashed.
    Failed,
    TimedOut,
    Cancelled,
}

impl RunOutcome {
    pub fn is_success(self) -> bool {
        matches!(self, Self::Clean | Self::Findings)
    }
}

/// Decide the outcome from what happened. `output` is the (redacted) combined output.
pub fn classify_outcome(exit_code: Option<i32>, timed_out: bool, cancelled: bool, rules: &ExitCodes, patterns: &[String], output: &str) -> AppResult<RunOutcome> {
    if cancelled {
        return Ok(RunOutcome::Cancelled);
    }
    if timed_out {
        return Ok(RunOutcome::TimedOut);
    }
    let Some(code) = exit_code else { return Ok(RunOutcome::Failed) };
    if rules.findings.contains(&code) {
        return Ok(RunOutcome::Findings);
    }
    if !rules.ok.contains(&code) {
        return Ok(RunOutcome::Failed);
    }
    for p in patterns {
        let re = RegexBuilder::new(p).size_limit(64 * 1024).build().map_err(|e| super::AppError::Validation(format!("findings pattern '{p}': {e}")))?;
        if re.is_match(output) {
            return Ok(RunOutcome::Findings);
        }
    }
    Ok(RunOutcome::Clean)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    /// The same id as the job that executed it.
    pub id: OperationId,
    pub tool_id: ToolId,
    pub tool_name: String,
    pub profile_id: String,
    pub profile_label: String,
    /// Installed version at run time, when known.
    pub tool_version: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    /// `None` while the run is still going.
    pub outcome: Option<RunOutcome>,
    /// The command as shown to the user (secrets masked).
    pub command: String,
    /// Canonical network targets, for the "what was tested" column.
    pub targets: Vec<String>,
    /// One-line scope summary, e.g. "your local practice lab" or "confirmed: public address".
    pub scope: String,
    pub network_activity: NetworkActivity,
    pub output_format: OutputFormat,
    /// Folder where the run could write result files.
    pub run_dir: Option<String>,
    /// Where the user saved the output, if they did.
    pub result_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PurgeReport {
    pub runs_deleted: usize,
    pub output_lines_deleted: usize,
    pub jobs_deleted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRow {
    pub seq: i64,
    pub stream: super::process::OutputStream,
    pub text: String,
}

/// Starting values for a launch form, with lab placeholders already replaced by real values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchDefaults {
    pub parameters: std::collections::HashMap<String, super::manifest::ParamValue>,
    /// Suggested folders for container mounts (the user still has to approve each one).
    pub mounts: std::collections::HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(ok: &[i32], findings: &[i32]) -> ExitCodes {
        ExitCodes { ok: ok.to_vec(), findings: findings.to_vec() }
    }

    #[test]
    fn exit_code_one_can_mean_findings_not_failure() {
        let r = rules(&[0], &[1]);
        assert_eq!(classify_outcome(Some(0), false, false, &r, &[], "").unwrap(), RunOutcome::Clean);
        assert_eq!(classify_outcome(Some(1), false, false, &r, &[], "").unwrap(), RunOutcome::Findings);
        assert_eq!(classify_outcome(Some(2), false, false, &r, &[], "").unwrap(), RunOutcome::Failed);
        assert_eq!(classify_outcome(Some(-1073741819), false, false, &r, &[], "").unwrap(), RunOutcome::Failed, "a crash code is a failure");
        assert!(RunOutcome::Findings.is_success() && RunOutcome::Clean.is_success());
        assert!(!RunOutcome::Failed.is_success() && !RunOutcome::TimedOut.is_success() && !RunOutcome::Cancelled.is_success());
    }

    #[test]
    fn patterns_detect_findings_when_the_exit_code_cannot() {
        let r = rules(&[0], &[]);
        let p = vec![r"(?m)^\d+/(tcp|udp)\s+open".to_string()];
        let out = "Starting Nmap\nPORT      STATE SERVICE\n18080/tcp open  http\n";
        assert_eq!(classify_outcome(Some(0), false, false, &r, &p, out).unwrap(), RunOutcome::Findings);
        assert_eq!(classify_outcome(Some(0), false, false, &r, &p, "PORT STATE SERVICE\n80/tcp closed http\n").unwrap(), RunOutcome::Clean);
        // A failing exit code stays a failure even if the output mentions an open port.
        assert_eq!(classify_outcome(Some(255), false, false, &r, &p, out).unwrap(), RunOutcome::Failed);
    }

    #[test]
    fn cancellation_and_timeouts_win_over_exit_codes() {
        let r = rules(&[0], &[1]);
        assert_eq!(classify_outcome(Some(0), false, true, &r, &[], "").unwrap(), RunOutcome::Cancelled);
        assert_eq!(classify_outcome(None, true, false, &r, &[], "").unwrap(), RunOutcome::TimedOut);
        assert_eq!(classify_outcome(None, false, false, &r, &[], "").unwrap(), RunOutcome::Failed, "no exit code means the process never completed");
    }

    #[test]
    fn invalid_patterns_are_errors_not_silent_clean_runs() {
        assert!(classify_outcome(Some(0), false, false, &rules(&[0], &[]), &["(".to_string()], "x").is_err());
    }
}
