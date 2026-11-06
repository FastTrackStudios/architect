//! Where an account's session lives: behind [`SecretStore`], so the
//! platform keychain is one store and memory another (tests, a web page).

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

/// What a secret store could not do.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The store is not there or refused (no keychain, a locked one, no
    /// Secret Service on the session bus). Its own words.
    #[error("the keychain: {0}")]
    Unavailable(String),
}

/// One secret per key: get, set, delete. Synchronous and `Send + Sync`, so
/// a store is shared between the app's flows and its actions.
pub trait SecretStore: Send + Sync {
    /// The secret under `key`, `None` when there is none.
    ///
    /// # Errors
    ///
    /// [`SecretError::Unavailable`] when the store cannot be read.
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;

    /// Keep `secret` under `key`, replacing what was there.
    ///
    /// # Errors
    ///
    /// [`SecretError::Unavailable`] when the store cannot be written.
    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError>;

    /// Forget `key`. Idempotent.
    ///
    /// # Errors
    ///
    /// [`SecretError::Unavailable`] when the store cannot be written.
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

/// Secrets in memory, for the process's life.
#[derive(Debug, Default)]
pub struct MemorySecrets {
    held: Mutex<BTreeMap<String, String>>,
}

impl MemorySecrets {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
        Ok(())
    }
}
