//! The platform keychain as a [`SecretStore`] (feature `keychain`): the
//! macOS and iOS `Keychain`, the Secret Service on Linux (GNOME Keyring,
//! `KWallet`, `KeePassXC`), the Windows Credential Manager.
//!
//! Each secret is an entry of the store's service name, its account the
//! key. The service is [`DEFAULT_SERVICE`] unless [`SERVICE_ENV`] names
//! another: a test or a throwaway session keeps its own entries apart from
//! the person's.

use crate::secrets::{SecretError, SecretStore};

/// The Architect app's entries.
pub const DEFAULT_SERVICE: &str = "app.fasttrackstudio.architect";

/// Names the keychain service instead of [`DEFAULT_SERVICE`].
pub const SERVICE_ENV: &str = "ARCHITECT_KEYCHAIN_SERVICE";

/// The platform keychain, under one service name.
#[derive(Clone, Debug)]
pub struct Keychain {
    service: String,
}

impl Keychain {
    /// The keychain's entries of `service`.
    #[must_use]
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// [`SERVICE_ENV`]'s service when set, else `default`.
    #[must_use]
    pub fn from_env(default: &str) -> Self {
        let service = std::env::var(SERVICE_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default.to_owned());
        Self::new(service)
    }

    /// Its service name.
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, key).map_err(unavailable)
    }
}

#[allow(clippy::needless_pass_by_value)] // as `map_err` hands it
fn unavailable(e: keyring::Error) -> SecretError {
    SecretError::Unavailable(e.to_string())
}

impl SecretStore for Keychain {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(unavailable(e)),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        self.entry(key)?.set_password(secret).map_err(unavailable)
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(unavailable(e)),
        }
    }
}
