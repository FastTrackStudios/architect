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

/// Secrets in a file only its owner can read: one JSON object, 0600.
///
/// What a development build and every test keep sessions in instead of
/// the platform keychain (`crate::secret_store`); a release build never
/// does. Each write goes to a fresh private temporary file beside it
/// (`tempfile`, never a fixed name a leftover or a symlink could take),
/// made 0600 on its handle, then renamed over it, under a lock file so
/// two processes writing at once each keep their own secret (read, change,
/// write all inside the lock). A file another user can read or write is
/// refused, never read. The folder is made 0700 only when this store made
/// it: an existing one (a `file:` path anywhere) is never changed.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct FileSecrets {
    path: std::path::PathBuf,
    lock: Mutex<()>,
}

/// How long a writer waits for another's lock before taking it as stale.
#[cfg(not(target_arch = "wasm32"))]
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

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

    fn fail(&self, e: &dyn std::fmt::Display) -> SecretError {
        SecretError::Unavailable(format!("{}: {e}", self.path.display()))
    }

    fn read(&self) -> Result<BTreeMap<String, String>, SecretError> {
        let meta = match std::fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(self.fail(&e)),
        };
        if !meta.is_file() {
            return Err(self.fail(&"not a plain file (a link or a folder): refused"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(self.fail(&"others may read or write it: refused (chmod 600 it)"));
            }
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| self.fail(&e))?;
        if text.trim().is_empty() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_str(&text).map_err(|e| self.fail(&e))
    }

    /// The folder, made (0700) when it is not there; an existing one is
    /// left as it is.
    fn folder(&self) -> Result<&std::path::Path, SecretError> {
        let dir = self
            .path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| self.fail(&e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| self.fail(&e))?;
            }
        }
        Ok(dir)
    }

    /// Runs `change` on the secrets with the file locked against other
    /// processes, and writes them back when it says so.
    fn update<T>(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, String>) -> (T, bool),
    ) -> Result<T, SecretError> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = self.folder()?;
        let _locked = FileLock::take(&self.path.with_extension("lock"), LOCK_WAIT)
            .map_err(|e| self.fail(&e))?;
        let mut all = self.read()?;
        let (out, changed) = change(&mut all);
        if changed {
            self.write(dir, &all)?;
        }
        Ok(out)
    }

    /// Writes `held` whole: a new private temporary file in `dir`, synced,
    /// renamed over the file.
    fn write(
        &self,
        dir: &std::path::Path,
        held: &BTreeMap<String, String>,
    ) -> Result<(), SecretError> {
        use std::io::Write as _;
        let text = serde_json::to_string(held).map_err(|e| self.fail(&e))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| self.fail(&e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            tmp.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| self.fail(&e))?;
        }
        tmp.write_all(text.as_bytes()).map_err(|e| self.fail(&e))?;
        tmp.as_file().sync_all().map_err(|e| self.fail(&e))?;
        tmp.persist(&self.path).map_err(|e| self.fail(&e.error))?;
        Ok(())
    }
}

/// A lock file held while it lives: made with `create_new` (so only one
/// holder), taken over when older than the wait (its holder died).
#[cfg(not(target_arch = "wasm32"))]
struct FileLock(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl FileLock {
    fn take(path: &std::path::Path, wait: std::time::Duration) -> std::io::Result<Self> {
        let start = std::time::Instant::now();
        loop {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(_) => return Ok(Self(path.to_path_buf())),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > wait);
                    if stale || start.elapsed() > wait {
                        let _ = std::fs::remove_file(path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SecretStore for FileSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.read()?.get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        self.update(|all| {
            let changed = all.get(key).map(String::as_str) != Some(secret);
            all.insert(key.to_owned(), secret.to_owned());
            ((), changed)
        })
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.update(|all| ((), all.remove(key).is_some()))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod file_tests {
    use std::sync::Arc;

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
            let mode =
                |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600, "the owner's alone");
            assert_eq!(mode(path.parent().unwrap()), 0o700, "the folder it made");
            let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.file_name())
                .collect();
            assert_eq!(
                leftovers.len(),
                1,
                "no temporary or lock file left: {leftovers:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_others_can_read_and_a_folder_it_did_not_make_are_left_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.path().join("secrets.json");
        // A planted or loosened file: refused, never read or extended.
        std::fs::write(&path, r#"{"a":"planted"}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let s = FileSecrets::new(&path);
        assert!(s.get("a").is_err());
        assert!(s.set("b", "two").is_err());
        // A planted temporary name or a link where the file goes buys nothing.
        std::fs::remove_file(&path).unwrap();
        let target = dir.path().join("elsewhere");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(s.get("a").is_err(), "a link is not followed");
        std::fs::remove_file(&path).unwrap();
        std::fs::write(path.with_extension("tmp"), "planted").unwrap();
        std::fs::set_permissions(
            path.with_extension("tmp"),
            std::fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        s.set("b", "two").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!target.exists());
        // The existing folder keeps its mode.
        assert_eq!(
            std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn two_writers_at_once_both_keep_their_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    // A store each, as two processes would have.
                    let s = Arc::new(FileSecrets::new(path));
                    for i in 0..10 {
                        s.set(&format!("k{n}-{i}"), "v").unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let s = FileSecrets::new(&path);
        for n in 0..8 {
            for i in 0..10 {
                assert!(
                    s.get(&format!("k{n}-{i}")).unwrap().is_some(),
                    "k{n}-{i} lost"
                );
            }
        }
    }
}
