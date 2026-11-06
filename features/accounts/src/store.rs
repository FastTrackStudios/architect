//! The accounts file and the sessions together: the file (no secrets) under
//! the app's folder, each account's session in a [`SecretStore`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use auth_client::{StoredSession, TokenStore, TokenStoreError};

use crate::model::{Account, AccountsFile};
use crate::secrets::{SecretError, SecretStore};

/// The accounts file's name in the app's folder.
pub const FILE: &str = "accounts.json";

/// What the store could not do.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("the accounts file: {0}")]
    File(String),
    #[error("no account {0}")]
    NoAccount(String),
}

/// Every account on this device: the file and the sessions.
pub struct AccountStore {
    path: Option<PathBuf>,
    secrets: Arc<dyn SecretStore>,
    file: Mutex<AccountsFile>,
}

impl std::fmt::Debug for AccountStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountStore")
            .field("path", &self.path)
            .field("accounts", &self.file().accounts.len())
            .finish_non_exhaustive()
    }
}

// The file's lock is held through its write, so two changes never write
// out of order.
#[allow(clippy::significant_drop_tightening)]
impl AccountStore {
    /// The store whose file is [`FILE`] in `dir`, its sessions in `secrets`.
    /// A missing or unreadable file is no accounts (signed out).
    #[must_use]
    pub fn open(dir: &Path, secrets: Arc<dyn SecretStore>) -> Self {
        let path = dir.join(FILE);
        let file = std::fs::read_to_string(&path)
            .ok()
            .and_then(|json| AccountsFile::from_json(&json).ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            secrets,
            file: Mutex::new(file),
        }
    }

    /// A store with no file: the accounts for the process's life.
    #[must_use]
    pub fn in_memory(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            path: None,
            secrets,
            file: Mutex::new(AccountsFile::default()),
        }
    }

    fn file(&self) -> MutexGuard<'_, AccountsFile> {
        self.file.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn save(&self, file: &AccountsFile) -> Result<(), StoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::File(e.to_string()))?;
        }
        let json = file.to_json().map_err(StoreError::File)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| StoreError::File(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| StoreError::File(e.to_string()))
    }

    /// The file as it is now.
    #[must_use]
    pub fn snapshot(&self) -> AccountsFile {
        self.file().clone()
    }

    /// The active account.
    #[must_use]
    pub fn active(&self) -> Option<Account> {
        self.file().active().cloned()
    }

    /// The session of the account `key`.
    ///
    /// # Errors
    ///
    /// The secret store's; a session it cannot read back is
    /// [`StoreError::File`].
    pub fn session(&self, key: &str) -> Result<Option<StoredSession>, StoreError> {
        let Some(json) = self.secrets.get(key)? else {
            return Ok(None);
        };
        serde_json::from_str(&json)
            .map(Some)
            .map_err(|e| StoreError::File(format!("a stored session: {e}")))
    }

    /// The active account's session.
    ///
    /// # Errors
    ///
    /// As [`Self::session`].
    pub fn active_session(&self) -> Result<Option<StoredSession>, StoreError> {
        let active = self.file().active.clone();
        active.map_or(Ok(None), |key| self.session(&key))
    }

    /// Keeps `session` for `account` and makes it the active account: the
    /// session goes into the secret store first, so a keychain that says no
    /// leaves the accounts as they were.
    ///
    /// # Errors
    ///
    /// The secret store's, or the file's.
    pub fn sign_in(&self, account: Account, session: &StoredSession) -> Result<(), StoreError> {
        let key = account.key();
        let json = serde_json::to_string(session)
            .map_err(|e| StoreError::File(format!("a session: {e}")))?;
        self.secrets.set(&key, &json)?;
        let mut file = self.file();
        let mut account = account;
        account.needs_sign_in = false;
        file.upsert_active(account);
        self.save(&file)
    }

    /// Replaces the session of `key` (a refresh), keeping it active or not.
    /// An account no longer on the device keeps nothing: a renewal that
    /// lands after its sign-out leaves no session behind.
    ///
    /// # Errors
    ///
    /// The secret store's.
    pub fn update_session(&self, key: &str, session: &StoredSession) -> Result<(), StoreError> {
        let json = serde_json::to_string(session)
            .map_err(|e| StoreError::File(format!("a session: {e}")))?;
        let file = self.file();
        if file.accounts.iter().any(|a| a.key() == key) {
            self.secrets.set(key, &json)?;
        }
        Ok(())
    }

    /// Makes `who` (a key, id or email) the active account.
    ///
    /// # Errors
    ///
    /// [`StoreError::NoAccount`] when no account is `who`; the file's.
    pub fn switch(&self, who: &str) -> Result<Account, StoreError> {
        let mut file = self.file();
        let account = file
            .find(who)
            .cloned()
            .ok_or_else(|| StoreError::NoAccount(who.to_owned()))?;
        file.active = Some(account.key());
        self.save(&file)?;
        Ok(account)
    }

    /// Takes `key` off this device: its session out of the secret store,
    /// the account off the file. The next account becomes active when it
    /// was.
    ///
    /// # Errors
    ///
    /// [`StoreError::NoAccount`]; the secret store's (the account stays);
    /// the file's.
    pub fn remove(&self, key: &str) -> Result<Account, StoreError> {
        if self.file().accounts.iter().all(|a| a.key() != key) {
            return Err(StoreError::NoAccount(key.to_owned()));
        }
        self.secrets.delete(key)?;
        let mut file = self.file();
        let gone = file
            .remove(key)
            .ok_or_else(|| StoreError::NoAccount(key.to_owned()))?;
        self.save(&file)?;
        Ok(gone)
    }

    /// Marks `key` as needing to sign in again (the issuer refused it), its
    /// session forgotten.
    ///
    /// # Errors
    ///
    /// The secret store's; the file's.
    pub fn expire(&self, key: &str) -> Result<(), StoreError> {
        self.secrets.delete(key)?;
        let mut file = self.file();
        if let Some(a) = file.accounts.iter_mut().find(|a| a.key() == key) {
            a.needs_sign_in = true;
        }
        self.save(&file)
    }

    /// Adds `account` without making it active (an update of its name or
    /// picture from the issuer).
    ///
    /// # Errors
    ///
    /// The file's.
    pub fn refresh_profile(&self, account: &Account) -> Result<(), StoreError> {
        let mut file = self.file();
        let key = account.key();
        if let Some(had) = file.accounts.iter_mut().find(|a| a.key() == key) {
            had.name.clone_from(&account.name);
            had.email.clone_from(&account.email);
            had.image.clone_from(&account.image);
        }
        self.save(&file)
    }
}

/// The active account's session as an [`auth_client::TokenStore`]: what an
/// org list (`architect_orgs::auth::fetch`) or a vox client
/// (`auth_client::TokenStoreMiddleware`) takes. It follows switches.
#[derive(Clone, Debug)]
pub struct ActiveSession(pub Arc<AccountStore>);

impl TokenStore for ActiveSession {
    fn save(&self, session: &StoredSession) -> Result<(), TokenStoreError> {
        let key = self
            .0
            .snapshot()
            .active
            .ok_or_else(|| TokenStoreError::Backend("no account is signed in".to_owned()))?;
        self.0
            .update_session(&key, session)
            .map_err(|e| TokenStoreError::Backend(e.to_string()))
    }

    fn load(&self) -> Result<Option<StoredSession>, TokenStoreError> {
        self.0
            .active_session()
            .map_err(|e| TokenStoreError::Backend(e.to_string()))
    }

    fn clear(&self) -> Result<(), TokenStoreError> {
        self.0.snapshot().active.map_or(Ok(()), |key| {
            self.0
                .remove(&key)
                .map(drop)
                .map_err(|e| TokenStoreError::Backend(e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySecrets;

    fn account(id: &str) -> Account {
        Account {
            id: id.to_owned(),
            issuer: "https://auth.example.test".to_owned(),
            email: Some(format!("{id}@example.test")),
            name: None,
            image: None,
            needs_sign_in: false,
        }
    }

    #[test]
    fn sessions_go_to_the_secret_store_and_never_to_the_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let secrets = Arc::new(MemorySecrets::new());
        let store = AccountStore::open(dir.path(), secrets.clone());
        let session = StoredSession::new("tok-secret-1").with_refresh_token("refresh-secret-1");
        store
            .sign_in(account("ada"), &session)
            .unwrap_or_else(|e| panic!("{e}"));
        let on_disk = std::fs::read_to_string(dir.path().join(FILE)).unwrap_or_default();
        assert!(on_disk.contains("ada@example.test"), "{on_disk}");
        assert!(!on_disk.contains("secret"), "{on_disk}");
        assert_eq!(secrets.len(), 1);
        // A second store over the same folder and keychain: still signed in.
        let again = AccountStore::open(dir.path(), secrets);
        assert_eq!(again.active().map(|a| a.id), Some("ada".to_owned()));
        let token = ActiveSession(Arc::new(again))
            .load()
            .ok()
            .flatten()
            .map(|s| s.token);
        assert_eq!(token.as_deref(), Some("tok-secret-1"));
    }

    #[test]
    fn switching_and_removing_keep_the_keychain_in_step() {
        let secrets = Arc::new(MemorySecrets::new());
        let store = AccountStore::in_memory(secrets.clone());
        store
            .sign_in(account("ada"), &StoredSession::new("a"))
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .sign_in(account("grace"), &StoredSession::new("g"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            store.switch("ada@example.test").map(|a| a.id).ok(),
            Some("ada".to_owned())
        );
        assert!(matches!(
            store.switch("nobody"),
            Err(StoreError::NoAccount(_))
        ));
        let key = account("ada").key();
        store.remove(&key).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(secrets.len(), 1);
        assert_eq!(store.active().map(|a| a.id), Some("grace".to_owned()));
        store
            .expire(&account("grace").key())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(secrets.is_empty());
        assert!(store.active().is_some_and(|a| a.needs_sign_in));
    }
}
