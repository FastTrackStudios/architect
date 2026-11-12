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

/// Secrets in a file only its owner can read.
///
/// 0600 in a 0700 folder on Unix, as one JSON object: what a development
/// build and every test keep sessions in instead of the platform keychain
/// (`crate::secret_store`).
///
/// A rebuilt, unsigned binary is a new code identity to macOS each time,
/// so a keychain entry an earlier build wrote makes the system ask the
/// person again on every read, and "Always Allow" cannot stick. A file has
/// no such prompt. It is no keychain: a release build keeps the keychain.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct FileSecrets {
    path: std::path::PathBuf,
    lock: Mutex<()>,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileSecrets {
    /// The secrets in `path` (made on the first write).
    #[must_use]
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    /// Where they are kept.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn read(&self) -> Result<BTreeMap<String, String>, SecretError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) if text.trim().is_empty() => Ok(BTreeMap::new()),
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| SecretError::Unavailable(format!("{}: {e}", self.path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(SecretError::Unavailable(format!("{}: {e}", self.path.display()))),
        }
    }

    /// Writes `held` whole: a private temporary file beside it, renamed
    /// over it.
    fn write(&self, held: &BTreeMap<String, String>) -> Result<(), SecretError> {
        use std::io::Write as _;
        let fail = |e: &dyn std::fmt::Display| {
            SecretError::Unavailable(format!("{}: {e}", self.path.display()))
        };
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        let text = serde_json::to_string(held).map_err(|e| fail(&e))?;
        let tmp = self.path.with_extension("tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp).map_err(|e| fail(&e))?;
        file.write_all(text.as_bytes()).map_err(|e| fail(&e))?;
        file.sync_all().map_err(|e| fail(&e))?;
        drop(file);
        std::fs::rename(&tmp, &self.path).map_err(|e| fail(&e))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SecretStore for FileSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.read()?.get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut all = self.read()?;
        all.insert(key.to_owned(), secret.to_owned());
        self.write(&all)
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut all = self.read()?;
        if all.remove(key).is_some() {
            self.write(&all)?;
        }
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod file_tests {
    use super::{FileSecrets, SecretStore as _};

    #[test]
    fn a_file_keeps_secrets_for_its_owner_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app").join("secrets.json");
        let s = FileSecrets::new(&path);
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "one").unwrap();
        s.set("b", "two").unwrap();
        // Another store on the same file sees them (a restart).
        let again = FileSecrets::new(&path);
        assert_eq!(again.get("a").unwrap().as_deref(), Some("one"));
        again.delete("a").unwrap();
        again.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
        assert_eq!(s.get("b").unwrap().as_deref(), Some("two"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the owner's alone");
        }
    }
}
