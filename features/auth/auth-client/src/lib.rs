//! Client-side session kit for architect-auth.
//!
//! Everything a downstream app needs to *hold on to* a session once the
//! auth service issued one, and to present it on later vox calls:
//!
//! - [`StoredSession`] — the token plus the metadata worth keeping
//!   around between runs (user id, email, expiry).
//! - [`TokenStore`] — save / load / clear of one stored session, behind
//!   a trait so the persistence strategy is swappable.
//! - [`FileTokenStore`] — native persistence: one JSON file, written
//!   atomically (temp file + rename) with `0600` permissions on unix.
//! - [`MemoryTokenStore`] — process-local store for wasm targets and
//!   tests, where there is no (sensible) filesystem.
//! - [`TokenStoreMiddleware`] — a [`vox::ClientMiddleware`] that loads
//!   the stored token on every outgoing call and attaches it exactly
//!   the way architect-auth's `AuthServerMiddleware` expects to find
//!   it: string metadata `authorization: Bearer <token>`.
//!
//! The crate is deliberately tiny and wasm-clean — no dependency on the
//! auth engine or proto crates. The only coupling is the wire format,
//! which the native integration tests round-trip against the real
//! `AuthServerMiddleware`.
//!
//! ```ignore
//! use std::sync::Arc;
//! use auth_client::{FileTokenStore, StoredSession, TokenStore, TokenStoreMiddleware};
//!
//! let store = Arc::new(FileTokenStore::new(dirs.data_dir().join("session.json")));
//!
//! // After login: persist the bundle's token.
//! store.save(&StoredSession::new(bundle.token.clone()))?;
//!
//! // Every typed vox client picks the session up automatically.
//! let tasks = tasks_client.with_middleware(TokenStoreMiddleware::new(store.clone()));
//!
//! // Logout: drop the file.
//! store.clear()?;
//! ```

use std::sync::{Arc, Mutex};

/// Metadata key carrying the session token on vox calls.
///
/// Must stay in sync with `auth_proto::AUTHORIZATION_METADATA_KEY` —
/// duplicated (rather than imported) so this crate stays free of the
/// proto crate's native-only dependency graph. The integration tests in
/// `features/auth/tests/native` pin the two together.
pub const AUTHORIZATION_METADATA_KEY: &str = "authorization";

/// A session as persisted by a [`TokenStore`]: the raw token plus the
/// metadata a client wants without a network round-trip.
///
/// All metadata is optional — [`StoredSession::new`] builds a bare
/// token entry, the `with_*` builders add what the caller knows (e.g.
/// from the `AuthSessionBundle` a login returned).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredSession {
    /// The raw session token, as returned at session creation time.
    pub token: String,
    /// The session's user id, stringified (`AuthUser::id`).
    #[serde(default)]
    pub user_id: Option<String>,
    /// The signed-in user's email, for display without a `whoami` call.
    #[serde(default)]
    pub email: Option<String>,
    /// Session expiry as unix seconds (UTC), mirroring
    /// `AuthSession::expires_at`. Informational: the server remains the
    /// authority on whether the token is still valid.
    #[serde(default)]
    pub expires_at_unix: Option<i64>,
}

impl StoredSession {
    /// A session entry holding just the token.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            user_id: None,
            email: None,
            expires_at_unix: None,
        }
    }

    /// Attach the user id.
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Attach the user's email.
    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    /// Attach the expiry (unix seconds, UTC).
    pub fn with_expires_at_unix(mut self, expires_at_unix: i64) -> Self {
        self.expires_at_unix = Some(expires_at_unix);
        self
    }
}

/// What can go wrong saving / loading a session.
#[derive(Debug, thiserror::Error)]
pub enum TokenStoreError {
    #[error("session store io: {0}")]
    Io(#[from] std::io::Error),
    #[error("session store encoding: {0}")]
    Encoding(#[from] serde_json::Error),
}

/// Save / load / clear of one session.
///
/// `Send + Sync` so a single store can be shared (via `Arc`) between
/// the app's login/logout flows and any number of
/// [`TokenStoreMiddleware`]-decorated clients. Methods are synchronous:
/// implementations are expected to be cheap (one tiny file, or memory).
pub trait TokenStore: Send + Sync {
    /// Persist `session`, replacing whatever was stored before.
    fn save(&self, session: &StoredSession) -> Result<(), TokenStoreError>;

    /// Load the stored session, `None` if nothing is stored.
    fn load(&self) -> Result<Option<StoredSession>, TokenStoreError>;

    /// Forget the stored session. Idempotent.
    fn clear(&self) -> Result<(), TokenStoreError>;
}

/// In-memory [`TokenStore`] — the wasm implementation, and the natural
/// choice for tests. On the web the token lives for the page's
/// lifetime; persisting across reloads is the embedding app's call
/// (and storage surface) — out of scope here.
#[derive(Debug, Default)]
pub struct MemoryTokenStore {
    session: Mutex<Option<StoredSession>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    fn save(&self, session: &StoredSession) -> Result<(), TokenStoreError> {
        *self.session.lock().expect("token store mutex poisoned") = Some(session.clone());
        Ok(())
    }

    fn load(&self) -> Result<Option<StoredSession>, TokenStoreError> {
        Ok(self
            .session
            .lock()
            .expect("token store mutex poisoned")
            .clone())
    }

    fn clear(&self) -> Result<(), TokenStoreError> {
        *self.session.lock().expect("token store mutex poisoned") = None;
        Ok(())
    }
}

/// File-backed [`TokenStore`] for native targets: one JSON document at
/// a caller-chosen path.
///
/// Writes are atomic — serialized to a sibling temp file first, then
/// `rename`d over the destination — so a crash mid-save never leaves a
/// truncated session file. On unix both the temp file and (through the
/// rename) the destination are created `0600`: the token is a bearer
/// credential and must not be world-readable.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct FileTokenStore {
    path: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileTokenStore {
    /// Store sessions at `path`. Parent directories are created on the
    /// first save.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn temp_path(&self) -> std::path::PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(".tmp");
        self.path.with_file_name(name)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TokenStore for FileTokenStore {
    fn save(&self, session: &StoredSession) -> Result<(), TokenStoreError> {
        use std::io::Write as _;

        let json = serde_json::to_vec_pretty(session)?;
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let temp = self.temp_path();
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        drop(file);

        std::fs::rename(&temp, &self.path)?;
        Ok(())
    }

    fn load(&self) -> Result<Option<StoredSession>, TokenStoreError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    fn clear(&self) -> Result<(), TokenStoreError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

/// Client middleware that presents the stored session on every call.
///
/// Loads from the [`TokenStore`] per call (so a re-login is picked up
/// without rebuilding clients) and, when a session is present, pushes
/// `authorization: Bearer <token>` string metadata — the exact shape
/// architect-auth's `AuthServerMiddleware` parses back out on the
/// server. The entry is flagged `SENSITIVE | NO_PROPAGATE`, matching
/// `AuthClientMiddleware` in the auth engine.
///
/// ```ignore
/// let client: TasksClient = local.establish().await?;
/// let client = client.with_middleware(TokenStoreMiddleware::new(store.clone()));
/// ```
#[derive(Clone)]
pub struct TokenStoreMiddleware {
    store: Arc<dyn TokenStore>,
}

impl TokenStoreMiddleware {
    pub fn new(store: Arc<dyn TokenStore>) -> Self {
        Self { store }
    }
}

impl vox::ClientMiddleware for TokenStoreMiddleware {
    fn pre<'a, 'call>(
        &'a self,
        _context: &'a vox::ClientContext<'a>,
        request: &'a mut vox::ClientRequest<'call, 'a>,
    ) -> vox::BoxMiddlewareFuture<'a> {
        // A missing or unreadable store means an unauthenticated call,
        // not a failed one — the server's middleware treats an absent
        // header the same way.
        let token = self
            .store
            .load()
            .ok()
            .flatten()
            .map(|session| session.token);
        Box::pin(async move {
            if let Some(token) = token {
                request.push_string_metadata(AUTHORIZATION_METADATA_KEY, format!("Bearer {token}"));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StoredSession {
        StoredSession::new("tok-123")
            .with_user_id("9f2c")
            .with_email("user@example.com")
            .with_expires_at_unix(4_102_444_800)
    }

    #[test]
    fn memory_store_round_trips_and_clears() {
        let store = MemoryTokenStore::new();
        assert_eq!(store.load().expect("load empty"), None);
        store.save(&sample()).expect("save");
        assert_eq!(store.load().expect("load"), Some(sample()));
        store.clear().expect("clear");
        assert_eq!(store.load().expect("load cleared"), None);
        store.clear().expect("clear is idempotent");
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn temp_session_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "auth-client-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_store_round_trips_creates_dirs_and_clears() {
        let dir = temp_session_path("nested");
        let path = dir.join("session.json");
        let store = FileTokenStore::new(&path);

        assert_eq!(store.load().expect("load missing file"), None);
        store.save(&sample()).expect("save creates parent dirs");
        assert_eq!(store.load().expect("load"), Some(sample()));

        // Overwrite goes through the same atomic path.
        store.save(&StoredSession::new("tok-456")).expect("resave");
        assert_eq!(
            store.load().expect("reload").map(|s| s.token),
            Some("tok-456".into())
        );

        store.clear().expect("clear");
        assert_eq!(store.load().expect("load cleared"), None);
        store.clear().expect("clear is idempotent");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn file_store_writes_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = temp_session_path("perms");
        let path = dir.join("session.json");
        let store = FileTokenStore::new(&path);
        store.save(&sample()).expect("save");

        let mode = std::fs::metadata(&path).expect("metadata").permissions();
        assert_eq!(mode.mode() & 0o777, 0o600, "session file must be 0600");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
