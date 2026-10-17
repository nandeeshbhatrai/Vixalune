//! Finds executables. A freshly installed tool is not on the PATH inherited by this process,
//! so on Windows the machine and user PATH are re-read from the registry (read-only; nothing is
//! ever written to PATH).

use crate::ports::ExecutableLocator;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct SystemExecutableLocator;

/// Expand `%NAME%` references using the current process environment.
pub fn expand_env_refs(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(windows)]
fn registry_path_dirs() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let mut dirs = Vec::new();
    let sources = [
        (HKEY_LOCAL_MACHINE, "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"),
        (HKEY_CURRENT_USER, "Environment"),
    ];
    for (hive, key) in sources {
        if let Ok(k) = RegKey::predef(hive).open_subkey(key) {
            if let Ok(path) = k.get_value::<String, _>("Path") {
                dirs.extend(path.split(';').filter(|s| !s.trim().is_empty()).map(|s| PathBuf::from(expand_env_refs(s.trim()))));
            }
        }
    }
    dirs
}

#[cfg(not(windows))]
fn registry_path_dirs() -> Vec<PathBuf> {
    Vec::new()
}

fn process_path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

/// `symlink_metadata` is used because Windows app-execution aliases (such as `winget.exe`) are
/// reparse points that `metadata` cannot follow.
fn is_runnable_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).map(|m| !m.is_dir()).unwrap_or(false)
}

/// Only plain executables are located; `.bat`/`.cmd` scripts are deliberately excluded because
/// running them would involve `cmd.exe` argument parsing.
fn valid_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty() && lower.ends_with(".exe") && !name.contains(['/', '\\', ':'])
}

impl ExecutableLocator for SystemExecutableLocator {
    fn locate(&self, file_name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf> {
        if !valid_file_name(file_name) {
            return None;
        }
        let mut dirs = registry_path_dirs();
        dirs.extend(process_path_dirs());
        dirs.extend(extra_dirs.iter().cloned());
        dirs.into_iter().filter(|d| d.is_absolute()).map(|d| d.join(file_name)).find(|c| is_runnable_file(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_variables_and_leaves_unknown_ones() {
        std::env::set_var("VIXALUNE_TEST_VAR", "C:\\X");
        assert_eq!(expand_env_refs("%VIXALUNE_TEST_VAR%\\bin"), "C:\\X\\bin");
        assert_eq!(expand_env_refs("%VIXALUNE_NOPE_NOT_SET%\\bin"), "%VIXALUNE_NOPE_NOT_SET%\\bin");
        assert_eq!(expand_env_refs("100%"), "100%");
    }

    #[test]
    fn rejects_scripts_paths_and_non_exe_names() {
        let l = SystemExecutableLocator;
        for bad in ["", "tool.bat", "tool.cmd", "tool", "..\\evil.exe", "a/b.exe", "C:\\x.exe", "x.exe:stream"] {
            assert!(l.locate(bad, &[]).is_none(), "{bad}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn finds_system_tools_and_extra_dirs() {
        let l = SystemExecutableLocator;
        assert!(l.locate("whoami.exe", &[]).is_some());
        let root = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        assert!(l.locate("notepad.exe", &[root]).is_some());
        assert!(l.locate("definitely-not-here.exe", &[]).is_none());
    }
}
