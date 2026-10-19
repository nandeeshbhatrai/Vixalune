//! Opens https URLs in the default browser without involving a shell.

use crate::domain::{AppError, AppResult};
use crate::ports::LinkOpener;
use url::Url;

#[derive(Default)]
pub struct SystemLinkOpener;

/// Absolute path of a program in the Windows folder. `Command::new("explorer.exe")` would search the application's own
/// folder first, so a program planted next to Vixalune could be started instead.
#[cfg(windows)]
fn windows_dir(exe: &str) -> AppResult<std::path::PathBuf> {
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).ok_or_else(|| AppError::Io("the Windows folder is not known".into()))?;
    let p = root.join(exe);
    if p.is_file() { Ok(p) } else { Err(AppError::NotFound(exe.into())) }
}

#[cfg(windows)]
fn system32(exe: &str) -> AppResult<std::path::PathBuf> {
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).ok_or_else(|| AppError::Io("the Windows folder is not known".into()))?;
    let p = root.join("System32").join(exe);
    if p.is_file() { Ok(p) } else { Err(AppError::NotFound(exe.into())) }
}

impl LinkOpener for SystemLinkOpener {
    fn open(&self, url: &str) -> AppResult<()> {
        let parsed = Url::parse(url).map_err(|e| AppError::Validation(e.to_string()))?;
        if parsed.scheme() != "https" {
            return Err(AppError::Validation("only https links can be opened".into()));
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            // Fixed program and argument array: the URL is a single argument, never parsed by a shell.
            std::process::Command::new(system32("rundll32.exe")?)
                .arg("url.dll,FileProtocolHandler")
                .arg(parsed.as_str())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()?;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err(AppError::NotImplemented("open link on this OS"))
        }
    }
    fn open_folder(&self, path: &std::path::Path) -> AppResult<()> {
        if !path.is_dir() {
            return Err(AppError::NotFound("that folder".into()));
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            // Fixed program; the folder is one argument and is not parsed by a shell.
            std::process::Command::new(windows_dir("explorer.exe")?).arg(path).creation_flags(CREATE_NO_WINDOW).spawn()?;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err(AppError::NotImplemented("open folder on this OS"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https() {
        assert!(SystemLinkOpener.open("file:///C:/Windows/System32/calc.exe").is_err());
        assert!(SystemLinkOpener.open("http://example.com").is_err());
        assert!(SystemLinkOpener.open("not a url").is_err());
    }
}
