//! Path safety. Manifests may only name *relative* sub-folders or a fixed set of well-known
//! location placeholders; nothing from a manifest is ever used as a raw absolute path.

use super::errors::{AppError, AppResult};
use std::path::{Component, Path, PathBuf};

const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn bad(msg: impl Into<String>) -> AppError {
    AppError::Validation(msg.into())
}

/// Validates a manifest-supplied relative path ("Vendor\\Tool"). Rejects absolute paths, drive
/// letters, UNC prefixes, `..`, alternate data streams, reserved device names, trailing dots or
/// spaces, control characters and over-long paths.
pub fn validate_relative(rel: &str) -> AppResult<()> {
    if rel.is_empty() || rel.len() > 120 {
        return Err(bad("path must be 1-120 characters"));
    }
    if rel.chars().any(|c| c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err(bad(format!("path '{rel}' contains forbidden characters")));
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(bad(format!("path '{rel}' must be relative")));
    }
    for part in rel.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." {
            return Err(bad(format!("path '{rel}' contains an empty, '.' or '..' segment")));
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return Err(bad(format!("path '{rel}' has a segment ending in '.' or space")));
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        if RESERVED.contains(&stem.as_str()) {
            return Err(bad(format!("path '{rel}' uses a reserved device name")));
        }
    }
    Ok(())
}

/// Joins `rel` under `base` after validation. The result is lexically guaranteed to stay inside `base`.
pub fn join_under(base: &Path, rel: &str) -> AppResult<PathBuf> {
    validate_relative(rel)?;
    let joined: PathBuf = base.join(rel.replace('/', "\\"));
    ensure_within(base, &joined)?;
    Ok(joined)
}

/// Lexical containment check (no filesystem access): `candidate` must be `base` or below it, with
/// no parent-directory components.
pub fn ensure_within(base: &Path, candidate: &Path) -> AppResult<()> {
    if candidate.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(bad("path escapes its base directory"));
    }
    if candidate.starts_with(base) {
        Ok(())
    } else {
        Err(bad(format!("'{}' is outside '{}'", candidate.display(), base.display())))
    }
}

/// Well-known folders a verification check may search. Resolved from the trusted OS environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownFolder {
    ProgramFiles,
    ProgramFilesX86,
    LocalAppData,
}

impl KnownFolder {
    fn placeholder(self) -> &'static str {
        match self {
            Self::ProgramFiles => "{ProgramFiles}",
            Self::ProgramFilesX86 => "{ProgramFilesX86}",
            Self::LocalAppData => "{LocalAppData}",
        }
    }
    fn env_var(self) -> &'static str {
        match self {
            Self::ProgramFiles => "ProgramFiles",
            Self::ProgramFilesX86 => "ProgramFiles(x86)",
            Self::LocalAppData => "LOCALAPPDATA",
        }
    }
    const ALL: [KnownFolder; 3] = [Self::ProgramFiles, Self::ProgramFilesX86, Self::LocalAppData];
}

/// Splits `"{ProgramFiles}\\Wireshark"` into (folder, "Wireshark") after validating the remainder.
pub fn parse_location(template: &str) -> AppResult<(KnownFolder, String)> {
    for f in KnownFolder::ALL {
        if let Some(rest) = template.strip_prefix(f.placeholder()) {
            let rest = rest.trim_start_matches(['/', '\\']);
            if !rest.is_empty() {
                validate_relative(rest)?;
            }
            return Ok((f, rest.to_string()));
        }
    }
    Err(bad(format!(
        "location '{template}' must start with {{ProgramFiles}}, {{ProgramFilesX86}} or {{LocalAppData}}"
    )))
}

/// Resolve a validated location template against the real environment. `None` if the variable is unset.
pub fn resolve_location(template: &str) -> AppResult<Option<PathBuf>> {
    let (folder, rest) = parse_location(template)?;
    Ok(std::env::var_os(folder.env_var()).map(|root| {
        let root = PathBuf::from(root);
        if rest.is_empty() {
            root
        } else {
            root.join(rest.replace('/', "\\"))
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_relative_paths() {
        for ok in ["nmap", "Vendor\\Tool", "vendor/tool-2", "a.b\\c_d"] {
            assert!(validate_relative(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn rejects_traversal_absolute_and_device_paths() {
        for bad in [
            "",
            "..",
            "..\\..\\Windows",
            "a\\..\\b",
            "a/../b",
            "C:\\Windows",
            "C:",
            "\\\\server\\share",
            "/etc/passwd",
            "\\Windows",
            "a\\",
            "a\\\\b",
            "con",
            "NUL.txt",
            "dir\\com1",
            "file.txt:stream",
            "trail.",
            "trail ",
            "a|b",
            "a\u{0}b",
            &"x".repeat(121),
        ] {
            assert!(validate_relative(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn join_under_stays_inside_base() {
        let base = Path::new("C:\\Tools");
        assert!(join_under(base, "nmap\\bin").unwrap().starts_with(base));
        assert!(join_under(base, "..\\evil").is_err());
        assert!(ensure_within(base, Path::new("C:\\Other\\x")).is_err());
        assert!(ensure_within(base, Path::new("C:\\Tools\\..\\x")).is_err());
    }

    #[test]
    fn locations_use_only_known_folders() {
        assert_eq!(parse_location("{ProgramFiles}\\Wireshark").unwrap(), (KnownFolder::ProgramFiles, "Wireshark".into()));
        assert!(parse_location("{ProgramFiles}").is_ok());
        assert!(parse_location("C:\\Program Files\\X").is_err());
        assert!(parse_location("{ProgramFiles}\\..\\Windows").is_err());
        assert!(parse_location("{SystemRoot}\\x").is_err());
        assert!(parse_location("%ProgramFiles%\\x").is_err());
    }
}
