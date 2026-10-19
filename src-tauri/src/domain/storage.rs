//! Where Vixalune keeps things, and which of those locations a user may change.
//!
//! Application data (settings, database, run folders, logs) lives in one data folder that is separate from the program
//! files, so installing, updating or removing the program never touches it unless the user asks.

use super::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;

/// Environment variable that chooses the data folder (portable or test installs).
pub const DATA_DIR_ENV: &str = "VIXALUNE_DATA_DIR";

/// Fixed address of the project's release page. The backend owns it; the web view cannot supply another.
pub const RELEASE_PAGE: &str = "https://github.com/nandeeshbhatrai/Vixalune/releases";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageInfo {
    pub data_dir: String,
    pub config_path: String,
    pub database_path: String,
    pub runs_dir: String,
    pub tools_dir: String,
    pub labs_dir: String,
    pub data_dir_overridden: bool,
    pub restart_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageUpdate {
    pub tools_dir: String,
    pub labs_dir: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    Tools,
    /// Practice material that scanners and containers must be able to read: AppData is refused.
    Labs,
}

fn bad(m: impl Into<String>) -> AppError {
    AppError::Validation(m.into())
}

const PROTECTED_TOP: &[&str] = &["windows", "program files", "program files (x86)", "programdata", "$recycle.bin", "system volume information", "recovery"];
const CREDENTIAL_FOLDERS: &[&str] = &[".ssh", ".aws", ".azure", ".gnupg", ".kube", ".docker"];

/// Validate a folder a user typed. Purely lexical (nothing is created or touched).
pub fn validate_storage_dir(raw: &str, kind: StorageKind) -> AppResult<PathBuf> {
    let p = raw.trim();
    if p.is_empty() || p.len() > 200 || p.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*' | ',')) {
        return Err(bad("the folder path is empty, too long, or contains forbidden characters"));
    }
    if p.starts_with("\\\\") || p.starts_with("//") {
        return Err(bad("network (UNC) and device paths cannot be used; use a full folder path such as D:\\Vixalune\\labs"));
    }
    let norm = p.replace('/', "\\");
    let b = norm.as_bytes();
    if !(b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') {
        return Err(bad("use a full folder path such as D:\\Vixalune\\labs"));
    }
    let rest = norm[3..].trim_end_matches('\\');
    if rest.is_empty() {
        return Err(bad("choose a specific folder, not a whole drive"));
    }
    let segs: Vec<&str> = rest.split('\\').collect();
    if segs.iter().any(|s| s.is_empty() || *s == "." || *s == ".." || s.ends_with('.') || s.ends_with(' ') || s.contains(':')) {
        return Err(bad("the folder path contains an empty, relative or invalid segment"));
    }
    let lower: Vec<String> = segs.iter().map(|s| s.to_ascii_lowercase()).collect();
    if PROTECTED_TOP.contains(&lower[0].as_str()) {
        return Err(bad(format!("'{}' is a protected system folder; choose a folder of your own", segs[0])));
    }
    if lower[0] == "users" && lower.len() <= 2 {
        return Err(bad("choose a folder inside your profile, not the profile folder itself"));
    }
    if let Some(s) = lower.iter().find(|s| CREDENTIAL_FOLDERS.contains(&s.as_str())) {
        return Err(bad(format!("folders named '{s}' hold credentials and cannot be used")));
    }
    if kind == StorageKind::Labs && lower.iter().any(|s| s == "appdata") {
        return Err(bad("choose a folder outside AppData: scanners and containers refuse to read inside it"));
    }
    let reserved = ["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "lpt1", "lpt2", "lpt3"];
    if lower.iter().any(|s| reserved.contains(&s.split('.').next().unwrap_or(""))) {
        return Err(bad("the folder name is reserved by Windows"));
    }
    Ok(PathBuf::from(format!("{}{}", &norm[..3].to_ascii_uppercase(), rest)))
}

/// Interpret `VIXALUNE_DATA_DIR`. `None` when unset; an invalid value is an error (never silently ignored, so data does not
/// end up somewhere unexpected).
pub fn data_dir_override(value: Option<OsString>) -> AppResult<Option<PathBuf>> {
    let Some(v) = value else { return Ok(None) };
    let s = v.to_string_lossy().to_string();
    if s.trim().is_empty() {
        return Ok(None);
    }
    validate_storage_dir(&s, StorageKind::Tools).map(Some).map_err(|e| bad(format!("{DATA_DIR_ENV} is not usable: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn accepts_ordinary_folders_and_normalises_them() {
        assert_eq!(validate_storage_dir("d:/Vixalune/labs", StorageKind::Labs).unwrap(), PathBuf::from("D:\\Vixalune\\labs"));
        assert_eq!(validate_storage_dir("  C:\\Users\\me\\Vixalune\\tools\\ ", StorageKind::Tools).unwrap(), PathBuf::from("C:\\Users\\me\\Vixalune\\tools"));
        assert!(validate_storage_dir("C:\\Users\\me\\AppData\\Local\\Vixalune\\tools", StorageKind::Tools).is_ok(), "tools may live in AppData");
    }

    #[test]
    fn refuses_unsafe_or_unusable_folders() {
        for bad in [
            "", "   ", "relative\\path", "D:", "D:\\", "\\\\server\\share\\x", "//server/share/x", "C:\\Windows\\x", "C:\\Program Files\\Vixalune", "C:\\ProgramData\\x", "C:\\Users\\me", "C:\\Users",
            "D:\\a\\..\\b", "D:\\a\\.\\b", "D:\\a\\\\b", "D:\\a\\b.", "D:\\a \\b","D:\\a:b", "D:\\x\\.ssh", "D:\\x\\.aws\\y", "D:\\con", "D:\\a|b", "D:\\a*b", "D:\\a,b", "D:\\a\u{0}b", "\u{202e}D:\\x",
        ] {
            assert!(validate_storage_dir(bad, StorageKind::Tools).is_err(), "{bad:?}");
        }
        assert!(validate_storage_dir(&format!("D:\\{}", "x".repeat(200)), StorageKind::Tools).is_err());
        assert!(validate_storage_dir("C:\\Users\\me\\AppData\\Roaming\\labs", StorageKind::Labs).is_err());
    }

    #[test]
    fn the_data_dir_override_is_validated_not_ignored() {
        assert_eq!(data_dir_override(None).unwrap(), None);
        assert_eq!(data_dir_override(Some("".into())).unwrap(), None);
        assert_eq!(data_dir_override(Some("D:\\Portable\\VixaluneData".into())).unwrap(), Some(PathBuf::from("D:\\Portable\\VixaluneData")));
        for bad in ["relative", "\\\\server\\share", "C:\\Windows\\x", "D:\\a\\..\\b"] {
            let e = data_dir_override(Some(bad.into())).unwrap_err().to_string();
            assert!(e.contains(DATA_DIR_ENV), "{bad}: {e}");
        }
    }

    proptest! {
        /// Whatever is accepted is a clean absolute drive path with no traversal and no protected top folder.
        #[test]
        fn accepted_folders_are_clean(s in "\\PC{0,60}") {
            if let Ok(p) = validate_storage_dir(&s, StorageKind::Labs) {
                let t = p.to_string_lossy().to_string();
                prop_assert!(t.as_bytes()[1] == b':' && t.as_bytes()[2] == b'\\');
                prop_assert!(!t.split('\\').any(|x| x == ".." || x == "."));
                prop_assert!(!t.chars().any(|c| c.is_control() || "<>\"|?*,".contains(c)));
                let top = t[3..].split('\\').next().unwrap().to_ascii_lowercase();
                prop_assert!(!PROTECTED_TOP.contains(&top.as_str()));
            }
        }
    }
}
