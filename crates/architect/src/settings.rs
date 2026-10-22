//! App settings on the device: a typed [`Store<T>`] holding one facet
//! value as JSON, and [`FileKv`], the directory-of-files key/value store
//! it sits on (native).
//!
//! # Where things live
//!
//! Every setting has a *key*, [`Store::key`] = `"{name}.json"`, the same
//! string on every target:
//!
//! - native: the file `<app dir>/<key>`, written through [`FileKv`]. The
//!   app dir is [`app_dir`]: `$<APP>_CONFIG_DIR` when set (the override for
//!   tests, portable installs and CI), otherwise `<config dir>/<app>`
//!   (`~/Library/Application Support/<app>` on macOS, `$XDG_CONFIG_HOME/<app>`
//!   on Linux, `%APPDATA%\<app>` on Windows).
//! - wasm: `localStorage["<app>/<key>"]` — the app name standing in for the
//!   app dir, so the two layouts read the same way ([`storage_key`]).
//!
//! # Writes
//!
//! A native write is atomic: the body goes to a temporary file in the same
//! directory, is synced, and is renamed over the target, so a crash leaves
//! either the old file or the new one and never a torn one. A write whose
//! body equals what is already on disk is skipped (no rename, no mtime
//! change, no file-watcher event). [`Store::private`] / [`FileKv::private`]
//! create the file `0600` on unix — for tokens and anything else that is
//! nobody else's business.
//!
//! [`FileKv`] also implements architect-atom's `Kv` when the `atom-persist`
//! feature is on, so `KvRows<FileKv>` is a native `LocalRows` with the
//! same key layout as the browser's `LocalStorageRows`.

use std::marker::PhantomData;

use facet::Facet;

/// Why a settings read or write failed.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The file (or the browser's storage) could not be read or written.
    #[error("settings i/o for {key}: {source}")]
    Io {
        key: String,
        #[source]
        source: std::io::Error,
    },
    /// The stored body is not a valid `T`.
    #[error("settings {key} is not valid: {message}")]
    Decode { key: String, message: String },
    /// The value could not be encoded as JSON.
    #[error("settings {key} could not be encoded: {message}")]
    Encode { key: String, message: String },
    /// No app dir: no `$<APP>_CONFIG_DIR` and the platform has no config dir.
    #[error("no config directory for app {0} (set {1})")]
    NoAppDir(String, String),
    /// The browser exposes no `localStorage`.
    #[error("no localStorage in this browser")]
    NoStorage,
}

/// The environment variable that overrides `app`'s settings directory:
/// `app` upper-cased, every non-alphanumeric byte as `_`, plus
/// `_CONFIG_DIR` (`"my-app"` → `MY_APP_CONFIG_DIR`).
#[must_use]
pub fn env_var(app: &str) -> String {
    let mut out: String = app
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    out.push_str("_CONFIG_DIR");
    out
}

/// The browser storage key for `key` of `app`: `"{app}/{key}"`. The same
/// relative layout as the native `<app dir>/<key>`.
#[must_use]
pub fn storage_key(app: &str, key: &str) -> String {
    format!("{app}/{key}")
}

/// `app`'s settings directory (not created): `$<APP>_CONFIG_DIR` when
/// set and non-empty, else `<platform config dir>/<app>`.
#[cfg(not(target_arch = "wasm32"))]
pub fn app_dir(app: &str) -> Result<std::path::PathBuf, SettingsError> {
    app_dir_from(app, |k| std::env::var_os(k), dirs::config_dir())
}

#[cfg(not(target_arch = "wasm32"))]
fn app_dir_from(
    app: &str,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
    config_dir: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, SettingsError> {
    let var = env_var(app);
    if let Some(dir) = env(&var).filter(|v| !v.is_empty()) {
        return Ok(dir.into());
    }
    config_dir
        .map(|d| d.join(app))
        .ok_or_else(|| SettingsError::NoAppDir(app.to_owned(), var))
}

// ── FileKv ──────────────────────────────────────────────────────────

/// A key/value store as a directory of files: key `k` is the file
/// `<dir>/<k>`, written atomically (temp file, sync, rename) and skipped
/// when unchanged.
///
/// Keys may hold any characters: bytes outside `[A-Za-z0-9._-]` (and a
/// leading `.`, so no key is hidden or `..`) are `%XX`-escaped in the file
/// name, so `KvRows`' `rows.<kind>.<scope>` keys map one-to-one onto files.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct FileKv {
    dir: std::path::PathBuf,
    private: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileKv {
    /// Files under `dir` (created on the first write).
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            private: false,
        }
    }

    /// Create files `0600` (unix; ignored elsewhere), and tighten an
    /// existing file to `0600` when it is next written.
    #[must_use]
    pub const fn private(mut self, private: bool) -> Self {
        self.private = private;
        self
    }

    /// The directory the files live in.
    #[must_use]
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// The file holding `key`.
    #[must_use]
    pub fn path(&self, key: &str) -> std::path::PathBuf {
        self.dir.join(encode_key(key))
    }

    /// The body under `key`, `Ok(None)` when there is none.
    pub fn read(&self, key: &str) -> std::io::Result<Option<String>> {
        match std::fs::read_to_string(self.path(key)) {
            Ok(body) => Ok(Some(body)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Store `body` under `key` atomically. Returns `false` (and touches
    /// nothing) when the file already holds exactly `body`.
    pub fn write(&self, key: &str, body: &str) -> std::io::Result<bool> {
        write_atomic(&self.path(key), body.as_bytes(), self.private)
    }

    /// Remove `key`; a missing key is not an error.
    pub fn delete(&self, key: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.path(key)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(all(feature = "atom-persist", not(target_arch = "wasm32")))]
impl architect_atom::Kv for FileKv {
    fn get(&self, key: &str) -> Option<String> {
        self.read(key).ok().flatten()
    }

    fn set(&self, key: &str, value: &str) -> Result<(), architect_atom::LocalError> {
        self.write(key, value)
            .map(|_| ())
            .map_err(|e| architect_atom::LocalError(format!("{}: {e}", self.path(key).display())))
    }

    fn remove(&self, key: &str) {
        let _ = self.delete(key);
    }
}

/// `key` as a file name: `[A-Za-z0-9_-]` and non-leading `.` kept, every
/// other byte `%XX`. Injective, so two keys never share a file.
#[cfg(not(target_arch = "wasm32"))]
fn encode_key(key: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(key.len());
    for (i, b) in key.bytes().enumerate() {
        let keep = b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || (b == b'.' && i > 0);
        if keep {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    if out.is_empty() {
        out.push('%');
    }
    out
}

/// Write `body` to `path` so a crash leaves the old file or the new one.
///
/// The body goes to a temp file in the same directory (same filesystem, so the rename is
/// atomic), `fsync`, rename over `path`, then `fsync` the directory
/// (unix). Skipped — returning `false` — when `path` already holds
/// `body`. `private` creates the temp file `0600` (unix).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_atomic(path: &std::path::Path, body: &[u8], private: bool) -> std::io::Result<bool> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    if std::fs::read(path).is_ok_and(|cur| cur == body) {
        if private {
            set_private(path)?;
        }
        return Ok(false);
    }
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(body)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(true)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
fn set_private(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
        if mode != 0o600 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

// ── Store<T> ────────────────────────────────────────────────────────

/// One typed settings value of an app, stored as facet JSON.
///
/// ```ignore
/// #[derive(Facet, Default, PartialEq)]
/// struct Prefs { theme: String, zoom: u32 }
///
/// let prefs: Store<Prefs> = Store::open("my-app", "prefs")?;
/// let mut p = prefs.load()?;          // Default when nothing is stored
/// p.zoom = 120;
/// prefs.save(&p)?;                     // atomic; a no-op when unchanged
/// ```
pub struct Store<T> {
    app: String,
    name: String,
    #[cfg(not(target_arch = "wasm32"))]
    kv: FileKv,
    _value: PhantomData<fn() -> T>,
}

impl<T> std::fmt::Debug for Store<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Store");
        d.field("app", &self.app).field("name", &self.name);
        #[cfg(not(target_arch = "wasm32"))]
        d.field("kv", &self.kv);
        d.finish_non_exhaustive()
    }
}

impl<T> Store<T> {
    /// The key, `"{name}.json"` on every target.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}.json", self.name)
    }

    /// The file this setting lives in.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn path(&self) -> std::path::PathBuf {
        self.kv.path(&self.key())
    }
}

impl<T> Store<T>
where
    T: Facet<'static> + Default,
{
    /// The setting `name` of `app`, in [`app_dir`] (native) or
    /// `localStorage` (wasm).
    pub fn open(app: impl Into<String>, name: impl Into<String>) -> Result<Self, SettingsError> {
        let app = app.into();
        #[cfg(not(target_arch = "wasm32"))]
        let kv = FileKv::new(app_dir(&app)?);
        Ok(Self {
            app,
            name: name.into(),
            #[cfg(not(target_arch = "wasm32"))]
            kv,
            _value: PhantomData,
        })
    }

    /// The setting `name` of `app`, in an explicit directory (native).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn in_dir(
        app: impl Into<String>,
        name: impl Into<String>,
        dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            app: app.into(),
            name: name.into(),
            kv: FileKv::new(dir),
            _value: PhantomData,
        }
    }

    /// Keep the file `0600` (unix). No effect on wasm.
    #[must_use]
    pub fn private(self) -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            kv: self.kv.private(true),
            ..self
        }
    }

    /// The stored value; `T::default()` when nothing is stored yet. A
    /// body that does not decode is an error, never silently replaced.
    pub fn load(&self) -> Result<T, SettingsError> {
        let key = self.key();
        self.read_body(&key)?.map_or_else(
            || Ok(T::default()),
            |body| {
                facet_json::from_str::<T>(&body).map_err(|e| SettingsError::Decode {
                    key,
                    message: e.to_string(),
                })
            },
        )
    }

    /// Store `value`. Returns `false` when the stored body was already
    /// identical (nothing was written).
    pub fn save(&self, value: &T) -> Result<bool, SettingsError> {
        let key = self.key();
        let mut body = facet_json::to_string_pretty(value).map_err(|e| SettingsError::Encode {
            key: key.clone(),
            message: e.to_string(),
        })?;
        body.push('\n');
        self.write_body(&key, &body)
    }

    /// Load, change, save — one call for the usual edit.
    pub fn update(&self, f: impl FnOnce(&mut T)) -> Result<T, SettingsError> {
        let mut value = self.load()?;
        f(&mut value);
        self.save(&value)?;
        Ok(value)
    }

    /// Forget the stored value (the next [`load`](Self::load) is the default).
    pub fn clear(&self) -> Result<(), SettingsError> {
        let key = self.key();
        #[cfg(not(target_arch = "wasm32"))]
        return self
            .kv
            .delete(&key)
            .map_err(|source| SettingsError::Io { key, source });
        #[cfg(target_arch = "wasm32")]
        {
            let storage = web_storage()?;
            let _ = storage.remove_item(&storage_key(&self.app, &key));
            Ok(())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_body(&self, key: &str) -> Result<Option<String>, SettingsError> {
        self.kv.read(key).map_err(|source| SettingsError::Io {
            key: key.to_owned(),
            source,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_body(&self, key: &str, body: &str) -> Result<bool, SettingsError> {
        self.kv
            .write(key, body)
            .map_err(|source| SettingsError::Io {
                key: key.to_owned(),
                source,
            })
    }

    #[cfg(target_arch = "wasm32")]
    fn read_body(&self, key: &str) -> Result<Option<String>, SettingsError> {
        let storage = web_storage()?;
        Ok(storage
            .get_item(&storage_key(&self.app, key))
            .ok()
            .flatten())
    }

    #[cfg(target_arch = "wasm32")]
    fn write_body(&self, key: &str, body: &str) -> Result<bool, SettingsError> {
        let storage = web_storage()?;
        let full = storage_key(&self.app, key);
        if storage.get_item(&full).ok().flatten().as_deref() == Some(body) {
            return Ok(false);
        }
        storage
            .set_item(&full, body)
            .map_err(|e| SettingsError::Io {
                key: key.to_owned(),
                source: std::io::Error::other(format!("{e:?}")),
            })?;
        Ok(true)
    }
}

#[cfg(target_arch = "wasm32")]
fn web_storage() -> Result<web_sys::Storage, SettingsError> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or(SettingsError::NoStorage)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn env_var_names() {
        assert_eq!(env_var("my-app"), "MY_APP_CONFIG_DIR");
        assert_eq!(env_var("fts.Task"), "FTS_TASK_CONFIG_DIR");
    }

    #[test]
    fn app_dir_prefers_env_override() {
        let env = |k: &str| (k == "MY_APP_CONFIG_DIR").then(|| OsString::from("/tmp/over"));
        let dir = app_dir_from("my-app", env, Some(PathBuf::from("/cfg"))).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/over"));

        let empty = |_: &str| Some(OsString::new());
        let dir = app_dir_from("my-app", empty, Some(PathBuf::from("/cfg"))).unwrap();
        assert_eq!(dir, PathBuf::from("/cfg/my-app"));

        let none = |_: &str| None;
        assert!(matches!(
            app_dir_from("my-app", none, None),
            Err(SettingsError::NoAppDir(_, v)) if v == "MY_APP_CONFIG_DIR"
        ));
    }

    #[test]
    fn keys_encode_injectively() {
        assert_eq!(encode_key("prefs.json"), "prefs.json");
        assert_eq!(encode_key("rows.task.org/1"), "rows.task.org%2F1");
        assert_eq!(encode_key(".."), "%2E.");
        assert_eq!(encode_key("a%b"), "a%25b");
        assert_ne!(encode_key("a/b"), encode_key("a%2Fb"));
        assert_eq!(encode_key(""), "%");
    }

    #[test]
    fn native_and_wasm_keys_line_up() {
        let dir = PathBuf::from("/cfg/my-app");
        let s: Store<u32> = Store::in_dir("my-app", "prefs", &dir);
        assert_eq!(s.key(), "prefs.json");
        assert_eq!(s.path(), dir.join("prefs.json"));
        assert_eq!(storage_key("my-app", &s.key()), "my-app/prefs.json");
    }
}
