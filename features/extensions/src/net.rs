//! The network, for extensions (#310): the host service's `fetch`, its
//! streamed bodies, and the secrets the host keeps for them.
//!
//! An extension never opens a socket: a WASI component's world has no
//! `wasi:sockets` or `wasi:http` (`crate::wasi` refuses one that imports
//! them), a Luau script and a page have no network of their own. They ask
//! the host to `fetch` (`crate::hostcall::Fetch`), and the host checks the
//! URL's host against `capabilities.hosts` as a person granted them
//! (`crate::hostcall::may_reach_url`), sends it, and follows each redirect
//! itself, checking every one the same way: a redirect to a host it may
//! not reach is refused, not followed. Each request has a time limit
//! ([`TIMEOUT`], at most [`MAX_TIMEOUT`]) and a size limit on what comes
//! back ([`MAX_BODY`] read whole; [`MAX_STREAM`] streamed); what it sends
//! is at most [`MAX_UPLOAD`].
//!
//! A response fetched with `stream` is kept by the host ([`Net`]) and read
//! a part at a time (`read_body`), by the extension that fetched it only;
//! at most [`MAX_STREAMS`] are open for one extension, and they go when it
//! unloads.
//!
//! **Secrets** ([`Secrets`]): what a manifest's `[[secrets]]` declare — an
//! API key the person enters once, asked for through the app's prompt
//! (`crate::hostui::HostUi::prompt`, a `secret` prompt) — kept by the host
//! in a file of its own (`<app dir>/extension-secrets.json` in the lab,
//! `0600`), never in the extension's folder, and never handed to it. A
//! request names one in a header's value (`"Bearer {{secret:api_key}}"`);
//! the host puts it in on a request to one of that secret's hosts, and
//! takes it off again on a redirect anywhere else.
//!
//! The requests run on a runtime of their own (two threads, `ext-net`),
//! so a fetch works from whichever executor its caller is on: a WASI
//! instance's thread, a Luau script's, a page's. The client is feature
//! `net` (reqwest): a build without it keeps secrets, and answers a fetch
//! `unsupported`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

#[cfg(feature = "net")]
mod client;
#[cfg(feature = "net")]
pub(crate) use client::{Got, Net, Plan, REDIRECT, send, whole};

/// How long a request waits for its response when it says nothing
/// (`timeout_ms`), and a streamed body for each part.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The longest a request may ask to wait.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(300);
/// The most of a body read whole (`max_bytes` may ask for less, not
/// more).
pub const MAX_BODY: u64 = 16 << 20;
/// The most of a streamed body, by default and at most.
pub const MAX_STREAM: u64 = 1 << 30;
/// The most a request may send.
pub const MAX_UPLOAD: usize = 16 << 20;
/// The most redirects one request follows.
pub const MAX_REDIRECTS: u32 = 10;
/// How much of a streamed body one `read_body` gives, when it says
/// nothing.
pub const CHUNK: u64 = 64 << 10;
/// The most streamed responses one extension has open.
pub const MAX_STREAMS: usize = 16;

/// The headers an extension may not set: the host's to say.
#[cfg(feature = "net")]
const HOST_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "upgrade",
];

/// The headers that go only to the host they were meant for: on a
/// redirect to another host they are left off (as a browser does).
#[cfg(feature = "net")]
const CREDENTIALS: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Secrets ─────────────────────────────────────────────────────────

/// The secrets the host keeps for extensions, by extension id and the
/// secret's id: in memory ([`Self::memory`]) or in one private file
/// ([`Self::open`]). Cheap to clone (the clones share them).
///
/// Its `Debug` shows which secrets are set, never a value.
#[derive(Clone, Default)]
pub struct Secrets {
    inner: Arc<SecretsInner>,
}

#[derive(Default)]
struct SecretsInner {
    file: Option<PathBuf>,
    kept: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set: BTreeMap<String, Vec<String>> = lock(&self.inner.kept)
            .iter()
            .map(|(id, s)| (id.clone(), s.keys().cloned().collect()))
            .collect();
        f.debug_struct("Secrets")
            .field("file", &self.inner.file)
            .field("set", &set)
            .finish()
    }
}

impl Secrets {
    /// Kept in memory, for the run.
    #[must_use]
    pub fn memory() -> Self {
        Self::default()
    }

    /// Kept in `file` (JSON, written `0600` on unix), read now when it is
    /// there. A file that does not read is taken as empty.
    #[must_use]
    pub fn open(file: impl Into<PathBuf>) -> Self {
        let file = file.into();
        let kept = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(SecretsInner {
                file: Some(file),
                kept: Mutex::new(kept),
            }),
        }
    }

    /// The file they are kept in, when they are.
    #[must_use]
    pub fn file(&self) -> Option<&Path> {
        self.inner.file.as_deref()
    }

    /// Whether extension `ext`'s secret `name` is set.
    #[must_use]
    pub fn is_set(&self, ext: &str, name: &str) -> bool {
        lock(&self.inner.kept)
            .get(ext)
            .is_some_and(|s| s.contains_key(name))
    }

    /// The secrets set for extension `ext`, by id.
    #[must_use]
    pub fn names(&self, ext: &str) -> Vec<String> {
        lock(&self.inner.kept)
            .get(ext)
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Sets extension `ext`'s secret `name` to `value`.
    ///
    /// # Errors
    ///
    /// Its file could not be written.
    pub fn set(&self, ext: &str, name: &str, value: &str) -> Result<(), String> {
        let mut kept = lock(&self.inner.kept);
        kept.entry(ext.to_owned())
            .or_default()
            .insert(name.to_owned(), value.to_owned());
        // Written while held: two sets never land out of order.
        let kept_now = self.keep(&kept);
        drop(kept);
        kept_now
    }

    /// Forgets extension `ext`'s secret `name`: whether it was set.
    ///
    /// # Errors
    ///
    /// Its file could not be written.
    pub fn forget(&self, ext: &str, name: &str) -> Result<bool, String> {
        let mut kept = lock(&self.inner.kept);
        let had = kept.get_mut(ext).is_some_and(|s| s.remove(name).is_some());
        if kept.get(ext).is_some_and(BTreeMap::is_empty) {
            kept.remove(ext);
        }
        let kept_now = self.keep(&kept);
        drop(kept);
        kept_now.map(|()| had)
    }

    /// Extension `ext`'s secret `name`'s value: the host's, to put into a
    /// request. Never an extension's to read.
    #[cfg(feature = "net")]
    pub(crate) fn get(&self, ext: &str, name: &str) -> Option<String> {
        lock(&self.inner.kept)
            .get(ext)
            .and_then(|s| s.get(name))
            .cloned()
    }

    fn keep(&self, kept: &BTreeMap<String, BTreeMap<String, String>>) -> Result<(), String> {
        let Some(file) = &self.inner.file else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(kept).map_err(|e| e.to_string())?;
        // A browser has no file to keep them in: in memory there.
        #[cfg(target_family = "wasm")]
        {
            let _ = (file, text);
            Ok(())
        }
        #[cfg(not(target_family = "wasm"))]
        architect::settings::write_atomic(file, text.as_bytes(), true)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", file.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::Secrets;

    #[test]
    fn secrets_are_kept_privately_and_never_shown() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.json");
        let s = Secrets::open(&file);
        s.set("lab.ai", "api_key", "sk-very-secret").unwrap();
        assert!(s.is_set("lab.ai", "api_key"));
        assert!(!format!("{s:?}").contains("sk-very-secret"), "{s:?}");
        let again = Secrets::open(&file);
        assert_eq!(
            again.get("lab.ai", "api_key").as_deref(),
            Some("sk-very-secret")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(again.forget("lab.ai", "api_key").unwrap());
        assert!(!Secrets::open(&file).is_set("lab.ai", "api_key"));
    }
}
