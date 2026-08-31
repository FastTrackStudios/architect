//! A [`TokenStore`] over the browser's `localStorage`.
//!
//! `auth-client` ships a memory store (lost on reload) and a file store
//! (no filesystem in a browser). A web app needs the third one, or
//! every refresh signs the user out.

use auth_client::{StoredSession, TokenStore, TokenStoreError};

/// Persists the session under one `localStorage` key.
///
/// Storage is origin-scoped by the browser, so a session saved by one
/// site is unreadable by another. It is still readable by any script on
/// *this* origin — which is the accepted tradeoff for a browser SPA, and
/// the reason the server keeps session lifetimes bounded.
#[derive(Clone, Debug)]
pub struct LocalStorageTokenStore {
    key: String,
}

impl LocalStorageTokenStore {
    /// A store under `key`. Namespace it per app so two FastTrackStudio
    /// apps on the same origin do not overwrite each other.
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }

    fn storage() -> Result<web_sys::Storage, TokenStoreError> {
        web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .ok_or_else(|| TokenStoreError::Backend("localStorage is unavailable".into()))
    }
}

impl Default for LocalStorageTokenStore {
    fn default() -> Self {
        Self::new("architect-auth.session")
    }
}

impl TokenStore for LocalStorageTokenStore {
    fn load(&self) -> Result<Option<StoredSession>, TokenStoreError> {
        let Some(raw) = Self::storage()?
            .get_item(&self.key)
            .map_err(|_| TokenStoreError::Backend("localStorage read failed".into()))?
        else {
            return Ok(None);
        };
        // A value we cannot parse is a value from an older format or a
        // corrupted write. Treat it as "no session" rather than an
        // error the caller has to special-case — the user just signs in
        // again.
        Ok(serde_json::from_str(&raw).ok())
    }

    fn save(&self, session: &StoredSession) -> Result<(), TokenStoreError> {
        let raw = serde_json::to_string(session)
            .map_err(|error| TokenStoreError::Backend(error.to_string()))?;
        Self::storage()?
            .set_item(&self.key, &raw)
            .map_err(|_| TokenStoreError::Backend("localStorage write failed".into()))
    }

    fn clear(&self) -> Result<(), TokenStoreError> {
        Self::storage()?
            .remove_item(&self.key)
            .map_err(|_| TokenStoreError::Backend("localStorage delete failed".into()))
    }
}
