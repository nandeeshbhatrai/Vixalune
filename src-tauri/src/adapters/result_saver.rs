//! Saving run output to a place the user chose.
//!
//! The location comes from the operating system's own "Save as" dialog, opened by the backend. The web view never
//! supplies a path or the content, and what the backend suggests is only a file name. The chosen path is still
//! validated: only plain document types are written, never into system or start-up folders, never to devices or
//! alternate data streams.

use crate::domain::{AppError, AppResult};
use crate::ports::ResultSaver;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

pub const ALLOWED_EXTENSIONS: &[&str] = &["txt", "log", "json", "xml", "html", "htm", "sarif", "csv", "md"];
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

const PROTECTED_SEGMENTS: &[&str] = &["windows", "system32", "syswow64", "program files", "program files (x86)", "programdata", "startup", "$recycle.bin", "system volume information"];
const RESERVED_NAMES: &[&str] = &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

fn bad(m: impl Into<String>) -> AppError {
    AppError::Validation(m.into())
}

/// A file name that is safe to *suggest*: letters, digits, `-`, `_`, `.` only, with an allowed extension.
pub fn suggested_file_name(tool: &str, profile: &str, started_at: &str, extension: &str) -> String {
    let clean = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(40).collect::<String>();
    let stamp: String = started_at.chars().filter(|c| c.is_ascii_digit()).take(12).collect();
    let ext = if ALLOWED_EXTENSIONS.contains(&extension) { extension } else { "txt" };
    format!("{}-{}-{}.{}", clean(tool), clean(profile), stamp, ext)
}

/// Check a destination chosen in the dialog. Returns the path to write.
pub fn validate_destination(path: &Path) -> AppResult<PathBuf> {
    let s = path.to_string_lossy().to_string();
    if s.starts_with("\\\\") || s.starts_with("//") {
        return Err(bad("saving to network or device paths is not allowed"));
    }
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| bad("the file name is not valid"))?;
    if name.is_empty() || name.len() > 150 || name.ends_with('.') || name.ends_with(' ') || name.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*' | ':')) {
        return Err(bad("the file name contains characters that are not allowed"));
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        return Err(bad("that file name is reserved by Windows"));
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(bad(format!("only these file types can be saved: {}", ALLOWED_EXTENSIONS.join(", "))));
    }
    // Only the drive letter may be followed by a colon; any other one is an alternate data stream.
    let after_drive = if s.len() >= 2 && s.as_bytes()[1] == b':' { &s[2..] } else { &s[..] };
    if after_drive.contains(':') {
        return Err(bad("the path contains a stream or device marker"));
    }
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| bad("choose a full path including the folder"))?;
    for comp in dir.components() {
        let c = comp.as_os_str().to_string_lossy().to_ascii_lowercase();
        if c == ".." {
            return Err(bad("the path contains '..'"));
        }
        if PROTECTED_SEGMENTS.contains(&c.as_str()) {
            return Err(bad(format!("'{c}' is a protected folder; choose another location such as Documents")));
        }
    }
    let canon_dir = std::fs::canonicalize(dir).map_err(|_| bad("the folder does not exist"))?;
    if !canon_dir.is_dir() {
        return Err(bad("the chosen location is not a folder"));
    }
    let canon_s = canon_dir.to_string_lossy().to_ascii_lowercase();
    if canon_s.starts_with("\\\\?\\unc\\") {
        return Err(bad("saving to network folders is not allowed"));
    }
    for seg in canon_s.split('\\') {
        if PROTECTED_SEGMENTS.contains(&seg) {
            return Err(bad(format!("'{seg}' is a protected folder; choose another location such as Documents")));
        }
    }
    if path.exists() && std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink() || !m.is_file()).unwrap_or(true) {
        return Err(bad("the chosen name is not a regular file"));
    }
    Ok(canon_dir.join(name))
}

/// Validate and write. Returns the path that was written.
pub fn write_result(path: &Path, content: &str) -> AppResult<PathBuf> {
    if content.len() > MAX_BYTES {
        return Err(bad("the output is too large to save"));
    }
    let target = validate_destination(path)?;
    std::fs::write(&target, content)?;
    Ok(target)
}

pub struct DialogSaver {
    app: AppHandle,
}

impl DialogSaver {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ResultSaver for DialogSaver {
    async fn save(&self, suggested_name: &str, content: &str) -> AppResult<Option<String>> {
        let app = self.app.clone();
        let name = suggested_name.to_string();
        let chosen = tauri::async_runtime::spawn_blocking(move || app.dialog().file().set_file_name(&name).add_filter("Text and report files", ALLOWED_EXTENSIONS).blocking_save_file())
            .await
            .map_err(|e| AppError::Io(e.to_string()))?;
        let Some(file) = chosen else { return Ok(None) };
        let path = file.into_path().map_err(|e| AppError::Io(e.to_string()))?;
        Ok(Some(write_result(&path, content)?.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_dir(name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("save-tests").join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn writes_allowed_files_to_existing_folders() {
        let d = project_dir("ok");
        let out = write_result(&d.join("scan.txt"), "hello").unwrap();
        assert_eq!(std::fs::read_to_string(out).unwrap(), "hello");
        assert!(write_result(&d.join("report.HTML"), "<p>x</p>").is_ok(), "extensions are case-insensitive");
        // Overwriting a regular file the user picked in the dialog is allowed.
        assert!(write_result(&d.join("scan.txt"), "again").is_ok());
    }

    #[test]
    fn refuses_dangerous_names_types_and_places() {
        let d = project_dir("bad");
        for name in ["run.exe", "run.bat", "run.ps1", "run.lnk", "run.dll", "run.js", "run", "run.txt.exe", "con.txt", "NUL.txt", "a|b.txt", "x.txt.", "run.txt:hidden", "run.txt:stream.exe"] {
            assert!(validate_destination(&d.join(name)).is_err(), "{name} must be refused");
        }
        assert!(validate_destination(&d.join("missing-folder").join("x.txt")).is_err());
        assert!(validate_destination(Path::new("\\\\server\\share\\x.txt")).is_err());
        assert!(validate_destination(Path::new("x.txt")).is_err(), "relative file names have no folder");
        assert!(validate_destination(&d.join("..").join("escape.txt")).is_err());
        for p in ["C:\\Windows\\x.txt", "C:\\Windows\\System32\\x.txt", "C:\\Program Files\\x.txt", "C:\\ProgramData\\x.txt"] {
            assert!(validate_destination(Path::new(p)).is_err(), "{p}");
        }
        assert!(validate_destination(&d).is_err(), "a folder is not a file name with an allowed type");
    }

    #[test]
    fn refuses_oversized_content() {
        let d = project_dir("big");
        assert!(write_result(&d.join("big.txt"), &"x".repeat(MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn suggested_names_are_plain() {
        assert_eq!(suggested_file_name("nmap", "quick-scan", "2026-10-19T10:30:45Z", "txt"), "nmap-quick-scan-202610191030.txt");
        assert_eq!(suggested_file_name("../x", "a b;c", "", "exe"), "x-abc-.txt");
    }
}
