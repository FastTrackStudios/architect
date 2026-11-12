//! The platform keychain as a [`SecretStore`] (feature `keychain`): the
//! macOS and iOS `Keychain`, the Secret Service on Linux (GNOME Keyring,
//! `KWallet`, `KeePassXC`), the Windows Credential Manager.
//!
//! Each secret is an entry of the store's service name, its account the
//! key. The service is [`DEFAULT_SERVICE`] unless [`SERVICE_ENV`] names
//! another, for a real, signed build.
//!
//! **Development builds and tests never touch it** ([`secret_store`]). An
//! unsigned binary rebuilt again and again is a new code identity to macOS
//! each time, so reading an entry an earlier build wrote asks the person
//! ("wants to use your confidential information"), on every launch, and
//! "Always Allow" cannot stick. So:
//!
//! - [`STORE_ENV`] picks the store: `file:<path>`, `file` (the app
//!   folder's `secrets.json`), `memory`, or `keychain`. The dev stack sets
//!   `file:` for its app.
//! - With none, a debug build on macOS or the iOS simulator keeps its
//!   sessions in the app folder's file ([`crate::FileSecrets`]) and says so
//!   once; a release build keeps the keychain.
//! - [`FORBID_ENV`] (`ARCHITECT_FORBID_KEYCHAIN=1`: every `cargo test`,
//!   `cargo nextest` and `cargo run` in the repo, `.cargo/config.toml`, and
//!   the dev stack) makes any keychain access panic, and [`secret_store`]
//!   choose the file instead.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::secrets::{FileSecrets, MemorySecrets, SecretError, SecretStore};

/// Picks the store: `file:<path>`, `file`, `memory` or `keychain`.
pub const STORE_ENV: &str = "ARCHITECT_SECRET_STORE";

/// `1`: this process must never open the platform keychain (tests, the dev
/// stack); [`Keychain`] panics if it is used.
pub const FORBID_ENV: &str = "ARCHITECT_FORBID_KEYCHAIN";

/// Whether [`FORBID_ENV`] forbids the keychain here.
#[must_use]
pub fn keychain_forbidden() -> bool {
    std::env::var(FORBID_ENV).is_ok_and(|v| v.trim() == "1")
}

/// The secrets file a store keeps in `dir` (the app's folder).
pub const SECRETS_FILE: &str = "secrets.json";

/// Which store a process uses ([`choose_store`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreChoice {
    /// The platform keychain, under this service.
    Keychain(String),
    /// A file only its owner reads.
    File(PathBuf),
    /// Memory, for the process's life.
    Memory,
}

/// Whether this build is a development one that the keychain would keep
/// asking about: a debug build on macOS or the iOS simulator.
#[must_use]
pub const fn development_build() -> bool {
    cfg!(debug_assertions)
        && (cfg!(target_os = "macos") || cfg!(all(target_os = "ios", target_abi = "sim")))
}

/// The store for a process, from its environment (`env`), whether the
/// keychain is forbidden and whether it is a development build: see the
/// module's docs. `dir` is the app's folder.
#[must_use]
pub fn choose_store(
    env: impl Fn(&str) -> Option<String>,
    default_service: &str,
    dir: &Path,
    forbidden: bool,
    development: bool,
) -> StoreChoice {
    let file = || StoreChoice::File(dir.join(SECRETS_FILE));
    let service = || {
        env(SERVICE_ENV)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default_service.to_owned())
    };
    match env(STORE_ENV).as_deref().map(str::trim) {
        Some("memory") => StoreChoice::Memory,
        Some("file") => file(),
        Some(v) if v.starts_with("file:") && v.len() > 5 => {
            StoreChoice::File(PathBuf::from(v.get(5..).unwrap_or_default()))
        }
        Some("keychain") if !forbidden => StoreChoice::Keychain(service()),
        _ if forbidden || development => file(),
        _ => StoreChoice::Keychain(service()),
    }
}

/// The store this process keeps sessions in ([`choose_store`] from its
/// environment); it says once when it is a file because this is a
/// development build or the keychain is forbidden.
#[must_use]
pub fn secret_store(default_service: &str, dir: &Path) -> Arc<dyn SecretStore> {
    let explicit = std::env::var(STORE_ENV).is_ok();
    match choose_store(
        |k| std::env::var(k).ok(),
        default_service,
        dir,
        keychain_forbidden(),
        development_build(),
    ) {
        StoreChoice::Keychain(service) => Arc::new(Keychain::new(service)),
        StoreChoice::File(path) => {
            if !explicit {
                tracing::info!(
                    path = %path.display(),
                    "accounts: a development build keeps its sessions in a file, not the keychain"
                );
            }
            Arc::new(FileSecrets::new(path))
        }
        StoreChoice::Memory => Arc::new(MemorySecrets::new()),
    }
}

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
        // A test or the dev stack opening the keychain would ask the person
        // at the screen, again and again: never.
        forbid_if_forbidden(&self.service);
        keyring::Entry::new(&self.service, key).map_err(unavailable)
    }
}

/// Panics when [`FORBID_ENV`] forbids the keychain: a test or the dev
/// stack reaching it is a bug to stop at once, not an error to carry.
#[allow(
    clippy::panic,
    reason = "a process forbidden the keychain must not reach it"
)]
fn forbid_if_forbidden(service: &str) {
    assert!(
        !keychain_forbidden(),
        "{FORBID_ENV}=1: this process must not open the platform keychain \
         (service {service}); use `secret_store` or `FileSecrets`"
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn development_builds_and_tests_keep_sessions_in_a_file() {
        let dir = Path::new("/app");
        let file = StoreChoice::File(dir.join(SECRETS_FILE));
        let kc = |s: &str| StoreChoice::Keychain(s.to_owned());
        // A release build: the keychain, its service as named.
        assert_eq!(choose_store(env(&[]), "svc", dir, false, false), kc("svc"));
        assert_eq!(
            choose_store(env(&[(SERVICE_ENV, "other")]), "svc", dir, false, false),
            kc("other")
        );
        // A development build: the file, unless it asks for the keychain.
        assert_eq!(choose_store(env(&[]), "svc", dir, false, true), file);
        assert_eq!(
            choose_store(env(&[(STORE_ENV, "keychain")]), "svc", dir, false, true),
            kc("svc")
        );
        // Forbidden: never the keychain, asked for or not.
        assert_eq!(choose_store(env(&[]), "svc", dir, true, false), file);
        assert_eq!(
            choose_store(env(&[(STORE_ENV, "keychain")]), "svc", dir, true, false),
            file
        );
        // Named stores.
        assert_eq!(
            choose_store(
                env(&[(STORE_ENV, "file:/tmp/s.json")]),
                "svc",
                dir,
                false,
                false
            ),
            StoreChoice::File(PathBuf::from("/tmp/s.json"))
        );
        assert_eq!(
            choose_store(env(&[(STORE_ENV, "file")]), "svc", dir, false, false),
            file
        );
        assert_eq!(
            choose_store(env(&[(STORE_ENV, "memory")]), "svc", dir, false, false),
            StoreChoice::Memory
        );
    }

    #[test]
    fn a_test_process_cannot_open_the_keychain() {
        // Every test runs with ARCHITECT_FORBID_KEYCHAIN=1 (.cargo/config.toml).
        assert!(keychain_forbidden(), "tests run with {FORBID_ENV}=1");
        let reached = std::panic::catch_unwind(|| {
            Keychain::new("app.fasttrackstudio.architect.test").get("k")
        });
        assert!(reached.is_err(), "the keychain is never opened from a test");
    }
}
