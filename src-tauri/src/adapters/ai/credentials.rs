//! API key storage in Windows Credential Manager.
//!
//! The operating system protects the stored value with the current user's Windows credentials (DPAPI); Vixalune adds no
//! encryption of its own. If Credential Manager cannot be used, every call fails: there is no file, database or settings
//! fallback. Credentials are named per provider and belong to the signed-in Windows user.

use crate::domain::ai::{AiError, AiErrorKind, AiResult};
use crate::domain::secret::Secret;
use crate::ports::CredentialStore;
use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "app.vixalune.desktop.ai";

#[cfg(windows)]
pub struct WindowsCredentialStore {
    service: String,
}

#[cfg(windows)]
impl Default for WindowsCredentialStore {
    fn default() -> Self {
        Self { service: SERVICE.into() }
    }
}

#[cfg(windows)]
impl WindowsCredentialStore {
    /// A store under another service name (used by the opt-in integration test so it never touches real keys).
    pub fn with_service(service: &str) -> Self {
        Self { service: service.into() }
    }

    fn entry(&self, provider: &str) -> AiResult<keyring::Entry> {
        if !crate::domain::ai::valid_provider_id(provider) {
            return Err(AiError::new(AiErrorKind::Configuration, "unknown provider"));
        }
        keyring::Entry::new(&self.service, provider).map_err(|e| store_error(&e))
    }
}

/// Error text for a failed Credential Manager call. It describes the failure, never the value.
#[cfg(windows)]
fn store_error(e: &keyring::Error) -> AiError {
    let why = match e {
        keyring::Error::NoStorageAccess(_) => "access to Windows Credential Manager was denied",
        keyring::Error::PlatformFailure(_) => "Windows Credential Manager reported an error",
        keyring::Error::BadEncoding(_) => "the stored credential could not be read",
        keyring::Error::TooLong(_, _) => "the credential is too long for Windows Credential Manager",
        _ => "Windows Credential Manager could not be used",
    };
    AiError::new(AiErrorKind::CredentialStore, why)
}

#[cfg(windows)]
impl CredentialStore for WindowsCredentialStore {
    fn set(&self, provider: &str, secret: &Secret) -> AiResult<()> {
        self.entry(provider)?.set_password(secret.expose()).map_err(|e| store_error(&e))
    }

    fn get(&self, provider: &str) -> AiResult<Option<Secret>> {
        match self.entry(provider)?.get_password() {
            Ok(v) => Ok(Some(Secret::new(v))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(&e)),
        }
    }

    fn delete(&self, provider: &str) -> AiResult<bool> {
        match self.entry(provider)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(store_error(&e)),
        }
    }

    fn exists(&self, provider: &str) -> AiResult<bool> {
        match self.entry(provider)?.get_password() {
            Ok(_) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(store_error(&e)),
        }
    }

    fn backend_name(&self) -> &'static str {
        "Windows Credential Manager (protected for your Windows account)"
    }
}

/// An in-memory store for tests and demonstrations. It is never wired into the application.
#[derive(Default)]
pub struct MemoryCredentialStore {
    map: Mutex<HashMap<String, String>>,
    /// When set, every call fails the way an unavailable Credential Manager would.
    pub unavailable: std::sync::atomic::AtomicBool,
}

impl MemoryCredentialStore {
    fn check(&self) -> AiResult<()> {
        if self.unavailable.load(std::sync::atomic::Ordering::SeqCst) {
            Err(AiError::new(AiErrorKind::CredentialStore, "Windows Credential Manager could not be used"))
        } else {
            Ok(())
        }
    }
}

impl CredentialStore for MemoryCredentialStore {
    fn set(&self, provider: &str, secret: &Secret) -> AiResult<()> {
        self.check()?;
        self.map.lock().unwrap().insert(provider.into(), secret.expose().to_string());
        Ok(())
    }
    fn get(&self, provider: &str) -> AiResult<Option<Secret>> {
        self.check()?;
        Ok(self.map.lock().unwrap().get(provider).map(|v| Secret::new(v.clone())))
    }
    fn delete(&self, provider: &str) -> AiResult<bool> {
        self.check()?;
        Ok(self.map.lock().unwrap().remove(provider).is_some())
    }
    fn exists(&self, provider: &str) -> AiResult<bool> {
        self.check()?;
        Ok(self.map.lock().unwrap().contains_key(provider))
    }
    fn backend_name(&self) -> &'static str {
        "in-memory test store"
    }
}

/// Used where no operating-system credential store is supported: every call fails, so keys cannot be saved anywhere.
#[cfg(not(windows))]
pub struct UnsupportedCredentialStore;

#[cfg(not(windows))]
impl CredentialStore for UnsupportedCredentialStore {
    fn set(&self, _: &str, _: &Secret) -> AiResult<()> {
        Err(unsupported())
    }
    fn get(&self, _: &str) -> AiResult<Option<Secret>> {
        Err(unsupported())
    }
    fn delete(&self, _: &str) -> AiResult<bool> {
        Err(unsupported())
    }
    fn exists(&self, _: &str) -> AiResult<bool> {
        Err(unsupported())
    }
    fn backend_name(&self) -> &'static str {
        "no secure credential storage on this system"
    }
}

#[cfg(not(windows))]
fn unsupported() -> AiError {
    AiError::new(AiErrorKind::CredentialStore, "secure credential storage is only available on Windows")
}

/// The credential store the application uses.
pub fn default_store() -> std::sync::Arc<dyn CredentialStore> {
    #[cfg(windows)]
    {
        std::sync::Arc::new(WindowsCredentialStore::default())
    }
    #[cfg(not(windows))]
    {
        std::sync::Arc::new(UnsupportedCredentialStore)
    }
}
