//! Windows host facts that are not process output: registry reads and a few documented Win32
//! calls. Everything here is read-only and works without administrator rights.

use crate::ports::HostFacts;
use std::path::Path;

#[derive(Default)]
pub struct WindowsHostFacts;

/// Strip any `user:password@` from a proxy string and keep `host:port` only.
pub fn proxy_host_only(raw: &str) -> Option<String> {
    let first = raw.split(';').next()?.trim();
    // "http=host:port" or "https=host:port" or plain "host:port" or "scheme://host:port"
    let after_eq = first.split_once('=').map(|(_, v)| v).unwrap_or(first);
    let no_scheme = after_eq.split_once("://").map(|(_, v)| v).unwrap_or(after_eq);
    let host = no_scheme.rsplit('@').next()?.trim().trim_end_matches('/');
    (!host.is_empty() && host.len() <= 255 && !host.chars().any(|c| c.is_whitespace() || c.is_control())).then(|| host.to_string())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    fn hklm() -> RegKey {
        RegKey::predef(HKEY_LOCAL_MACHINE)
    }

    impl HostFacts for WindowsHostFacts {
        fn windows(&self) -> (Option<u32>, Option<String>, Option<String>) {
            let Ok(k) = hklm().open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion") else { return (None, None, None) };
            let build = k.get_value::<String, _>("CurrentBuildNumber").ok().and_then(|s| s.trim().parse().ok());
            (build, k.get_value::<String, _>("DisplayVersion").ok(), k.get_value::<String, _>("EditionID").ok())
        }

        fn architecture(&self) -> String {
            match std::env::consts::ARCH {
                "x86_64" => "x64".into(),
                "aarch64" => "arm64".into(),
                other => other.into(),
            }
        }

        fn firmware_virtualization(&self) -> Option<bool> {
            // PF_VIRT_FIRMWARE_ENABLED = 21
            Some(unsafe { windows_sys::Win32::System::Threading::IsProcessorFeaturePresent(21) } != 0)
        }

        fn hypervisor_present(&self) -> Option<bool> {
            // CPUID leaf 1, ECX bit 31 is set when a hypervisor is running (Windows with WSL 2 /
            // Hyper-V / Virtual Machine Platform runs as a guest of its own hypervisor).
            #[cfg(target_arch = "x86_64")]
            {
                Some(std::arch::x86_64::__cpuid(1).ecx >> 31 & 1 == 1)
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                None
            }
        }

        fn disk(&self, path: &Path) -> Option<(u64, u64)> {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
            // Walk up to an existing folder so a not-yet-created tools dir still resolves its volume.
            let mut p = path.to_path_buf();
            while !p.exists() {
                if !p.pop() {
                    return None;
                }
            }
            let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
            let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
            (ok != 0).then_some((free, total))
        }

        fn pending_restart(&self) -> Option<bool> {
            let keys = [
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate\\Auto Update\\RebootRequired",
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Component Based Servicing\\RebootPending",
            ];
            Some(keys.iter().any(|k| hklm().open_subkey(k).is_ok()))
        }

        fn proxy(&self) -> Option<String> {
            let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings").ok()?;
            let enabled: u32 = k.get_value("ProxyEnable").unwrap_or(0);
            if enabled == 0 {
                return None;
            }
            proxy_host_only(&k.get_value::<String, _>("ProxyServer").ok()?)
        }

        fn policy_keys(&self) -> Vec<String> {
            ["SOFTWARE\\Policies\\Microsoft\\Windows\\WSL", "SOFTWARE\\Policies\\Microsoft\\Windows\\HyperV"]
                .iter()
                .filter(|k| hklm().open_subkey(k).is_ok())
                .map(|k| format!("HKLM\\{k}"))
                .collect()
        }

        fn is_elevated(&self) -> Option<bool> {
            use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
            use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
            unsafe {
                let mut token: HANDLE = std::ptr::null_mut();
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                    return None;
                }
                let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
                let mut ret = 0u32;
                let ok = GetTokenInformation(token, TokenElevation, &mut elevation as *mut _ as *mut _, std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut ret);
                CloseHandle(token);
                (ok != 0).then_some(elevation.TokenIsElevated != 0)
            }
        }

        fn docker_host_env(&self) -> Option<String> {
            std::env::var("DOCKER_HOST").ok().filter(|s| !s.trim().is_empty())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    impl HostFacts for WindowsHostFacts {
        fn windows(&self) -> (Option<u32>, Option<String>, Option<String>) {
            (None, None, None)
        }
        fn architecture(&self) -> String {
            std::env::consts::ARCH.into()
        }
        fn firmware_virtualization(&self) -> Option<bool> {
            None
        }
        fn hypervisor_present(&self) -> Option<bool> {
            None
        }
        fn disk(&self, _: &Path) -> Option<(u64, u64)> {
            None
        }
        fn pending_restart(&self) -> Option<bool> {
            None
        }
        fn proxy(&self) -> Option<String> {
            None
        }
        fn policy_keys(&self) -> Vec<String> {
            Vec::new()
        }
        fn is_elevated(&self) -> Option<bool> {
            None
        }
        fn docker_host_env(&self) -> Option<String> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_strings_lose_credentials_and_schemes() {
        assert_eq!(proxy_host_only("proxy.corp:8080").as_deref(), Some("proxy.corp:8080"));
        assert_eq!(proxy_host_only("http=proxy.corp:8080;https=proxy.corp:8443").as_deref(), Some("proxy.corp:8080"));
        assert_eq!(proxy_host_only("http://user:s3cret@proxy.corp:3128/").as_deref(), Some("proxy.corp:3128"));
        assert_eq!(proxy_host_only(""), None);
        assert!(!proxy_host_only("http://bob:hunter2@p:1").unwrap().contains("hunter2"));
    }

    #[cfg(windows)]
    #[test]
    fn reads_real_facts_without_admin() {
        let f = WindowsHostFacts;
        let (build, _, _) = f.windows();
        assert!(build.is_some_and(|b| b >= 19041), "{build:?}");
        assert!(matches!(f.architecture().as_str(), "x64" | "arm64"));
        let (free, total) = f.disk(Path::new("C:\\")).expect("disk space of C:");
        assert!(total > 0 && free <= total);
        assert!(f.disk(Path::new("C:\\definitely\\not\\created\\yet")).is_some(), "falls back to an existing parent");
        assert!(f.firmware_virtualization().is_some());
        assert!(f.pending_restart().is_some());
        assert!(f.is_elevated().is_some());
    }
}


