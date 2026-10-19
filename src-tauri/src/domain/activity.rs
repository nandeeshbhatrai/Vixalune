//! Activity events and secret redaction. Redaction is applied by the logger before anything
//! is persisted, so a caller cannot forget it.

use super::ids::ToolId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    AppStarted,
    EnvironmentChecked,
    InstallStarted,
    InstallSucceeded,
    InstallFailed,
    Uninstalled,
    Verified,
    RunStarted,
    RunFinished,
    RunTerminated,
    LabStarted,
    LabStopped,
    SettingsChanged,
    ResultSaved,
    HistoryPurged,
    SetupStarted,
    SetupSucceeded,
    SetupFailed,
    RestartRequested,
    RestartCancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub at: String, // RFC 3339, UTC
    pub kind: ActivityKind,
    pub tool_id: Option<ToolId>,
    pub summary: String,
}

const SENSITIVE_KEYS: &[&str] = &[
    "password", "passwd", "pwd", "pass", "token", "secret", "apikey", "api-key", "api_key", "authorization", "auth", "cookie",
    "key", "credential", "credentials",
];

const MASK: &str = "[REDACTED]";

fn key_is_sensitive(raw: &str) -> bool {
    let k = raw.trim_start_matches('-').to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|s| k == *s || k.ends_with(&format!("-{s}")) || k.ends_with(&format!("_{s}")))
}

/// Best-effort masking of secrets in free text: `key=value`, `--key value`, `Bearer <x>`,
/// and `user:pass@host` URL credentials. This is a safety net; callers should also avoid
/// logging full command lines and parameter values in the first place.
pub fn redact(input: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut mask_next = false;
    let mut after_bearer = false;
    for tok in input.split_whitespace() {
        if mask_next || after_bearer {
            out.push(MASK.into());
            mask_next = false;
            after_bearer = false;
            continue;
        }
        if tok.eq_ignore_ascii_case("bearer") || tok.eq_ignore_ascii_case("basic") {
            out.push(tok.into());
            after_bearer = true;
            continue;
        }
        if let Some((k, _)) = tok.split_once('=').or_else(|| tok.split_once(':').filter(|(k, v)| !v.is_empty() && !v.starts_with("//") && !k.contains('/'))) {
            if key_is_sensitive(k) {
                let sep = if tok.contains('=') { '=' } else { ':' };
                out.push(format!("{k}{sep}{MASK}"));
                continue;
            }
        }
        if tok.starts_with('-') && key_is_sensitive(tok) {
            out.push(tok.into());
            mask_next = true;
            continue;
        }
        if let Some(scheme_end) = tok.find("://") {
            let rest = &tok[scheme_end + 3..];
            if let Some(at) = rest.find('@') {
                if rest[..at].contains(':') {
                    out.push(format!("{}{}{}", &tok[..scheme_end + 3], MASK, &rest[at..]));
                    continue;
                }
            }
        }
        out.push(tok.into());
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_key_value_pairs() {
        assert_eq!(redact("login password=hunter2 user=bob"), "login password=[REDACTED] user=bob");
        assert_eq!(redact("API_KEY=abc123"), "API_KEY=[REDACTED]");
    }

    #[test]
    fn masks_flag_values() {
        assert_eq!(redact("tool --password hunter2 --host x"), "tool --password [REDACTED] --host x");
        assert_eq!(redact("tool --api-key abc"), "tool --api-key [REDACTED]");
    }

    #[test]
    fn masks_bearer_and_url_credentials() {
        assert_eq!(redact("Authorization: Bearer abc.def"), "Authorization: Bearer [REDACTED]");
        assert_eq!(redact("https://bob:pw@example.com/x"), "https://[REDACTED]@example.com/x");
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        assert_eq!(redact("Installed nmap 7.95 successfully"), "Installed nmap 7.95 successfully");
    }
}

