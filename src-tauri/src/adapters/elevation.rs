//! Running exactly one fixed command with administrator rights.
//!
//! Vixalune itself never runs elevated. For the few supported setup steps (installing WSL) it asks
//! Windows to launch **one** program with the `runas` verb; Windows shows its own consent (UAC)
//! prompt and the user can decline. Nothing stays elevated afterwards, and the arguments are
//! restricted to a tiny safe character set so no quoting or injection is possible.
//!
//! Output of an elevated process cannot be captured, so only the exit code is available.

use crate::domain::{AppError, AppResult};
use crate::ports::{ElevatedOutcome, ElevatedRunner};
use async_trait::async_trait;
use std::path::Path;
use std::time::Duration;

#[derive(Default)]
pub struct UacElevatedRunner;

/// Arguments for an elevated command: letters, digits and `-_.:/\` only. No spaces, quotes or shell characters.
pub fn validate_elevated_args(args: &[String]) -> AppResult<()> {
    for a in args {
        let ok = !a.is_empty() && a.len() <= 64 && a.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '\\'));
        if !ok {
            return Err(AppError::Validation(format!("refusing unsafe argument '{a}' for an elevated command")));
        }
    }
    Ok(())
}

/// The only programs that may be started with administrator rights live in the Windows System32 folder. A program from
/// anywhere else (the PATH, the application folder, a download) is refused even if every other check passed.
pub fn is_system32_program(program: &Path) -> bool {
    let Some(root) = std::env::var_os("SystemRoot") else { return false };
    let dir = Path::new(&root).join("System32");
    program.is_absolute()
        && program.parent().is_some_and(|p| p.as_os_str().eq_ignore_ascii_case(dir.as_os_str()))
        && !program.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

#[async_trait]
impl ElevatedRunner for UacElevatedRunner {
    #[cfg(windows)]
    async fn run(&self, program: &Path, args: &[String], timeout: Duration) -> AppResult<ElevatedOutcome> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

        validate_elevated_args(args)?;
        if !is_system32_program(program) {
            return Err(AppError::Validation(format!("elevated program '{}' must be a file in the Windows System32 folder", program.display())));
        }
        if !program.is_absolute() || !program.is_file() {
            return Err(AppError::Validation(format!("elevated program '{}' must be an existing absolute path", program.display())));
        }
        let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain(std::iter::once(0)).collect() };
        let verb = wide("runas".as_ref());
        let file = wide(program.as_os_str());
        let params = wide(args.join(" ").as_ref());
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);

        tokio::task::spawn_blocking(move || {
            // SAFETY: all pointers refer to NUL-terminated buffers that outlive the call.
            unsafe {
                let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
                info.fMask = SEE_MASK_NOCLOSEPROCESS;
                info.lpVerb = verb.as_ptr();
                info.lpFile = file.as_ptr();
                info.lpParameters = params.as_ptr();
                info.nShow = 0; // SW_HIDE
                if ShellExecuteExW(&mut info) == 0 {
                    let code = GetLastError();
                    return if code == ERROR_CANCELLED {
                        Ok(ElevatedOutcome { exit_code: None, declined: true })
                    } else {
                        Err(AppError::Io(format!("Windows could not start the elevated command (error {code})")))
                    };
                }
                let handle = info.hProcess;
                if handle.is_null() {
                    return Err(AppError::Io("Windows did not return a process handle".into()));
                }
                let wait = WaitForSingleObject(handle, ms);
                let result = if wait == WAIT_OBJECT_0 {
                    let mut code = 0u32;
                    if GetExitCodeProcess(handle, &mut code) != 0 {
                        Ok(ElevatedOutcome { exit_code: Some(code as i32), declined: false })
                    } else {
                        Err(AppError::Io("could not read the exit code of the elevated command".into()))
                    }
                } else {
                    // The elevated process cannot be killed from here; say so rather than pretend.
                    Err(AppError::TimedOut("the elevated command is still running; it was not stopped".into()))
                };
                CloseHandle(handle);
                result
            }
        })
        .await
        .map_err(|e| AppError::Io(e.to_string()))?
    }

    #[cfg(not(windows))]
    async fn run(&self, _: &Path, _: &[String], _: Duration) -> AppResult<ElevatedOutcome> {
        Err(AppError::NotImplemented("elevated commands on this OS"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_arguments_cannot_carry_shell_syntax_or_spaces() {
        let ok = ["--install", "--no-distribution", "--distribution", "Ubuntu-24.04"].map(String::from);
        assert!(validate_elevated_args(&ok).is_ok());
        for evil in ["a b", "a;b", "a&b", "a|b", "\"x\"", "$(x)", "`x`", "a>b", "", "%PATH%", &"x".repeat(65)] {
            assert!(validate_elevated_args(&[evil.to_string()]).is_err(), "{evil:?}");
        }
    }

    #[test]
    fn only_programs_in_the_system_folder_may_be_elevated() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        assert!(is_system32_program(&Path::new(&root).join("System32").join("wsl.exe")));
        for bad in ["wsl.exe", r"C:\Users\me\AppData\Local\Temp\wsl.exe", r"C:\Program Files\Docker\docker.exe"] {
            assert!(!is_system32_program(Path::new(bad)), "{bad}");
        }
        assert!(!is_system32_program(&Path::new(&root).join("System32").join("sub").join("x.exe")), "only the folder itself, not subfolders");
        assert!(!is_system32_program(&Path::new(&root).join("System32").join("..").join("evil").join("x.exe")));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn refuses_relative_or_missing_programs_before_any_prompt() {
        let r = UacElevatedRunner;
        assert!(r.run(Path::new("wsl.exe"), &[], Duration::from_secs(1)).await.is_err());
        assert!(r.run(Path::new("C:\\definitely\\missing.exe"), &[], Duration::from_secs(1)).await.is_err());
        assert!(r.run(Path::new("C:\\Windows\\System32\\wsl.exe"), &["a b".to_string()], Duration::from_secs(1)).await.is_err());
    }
}
