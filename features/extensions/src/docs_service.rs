//! The host's side of the documents service (`architect-documents`, #706).
//!
//! The host reaches the app's documents and each extension's data only
//! through the generated `DocumentsClient`, served in-process
//! (`architect::serve_local`) on a small runtime of its own.
//!
//! What it serves is the one implementation, `DocumentsServer`, over:
//!
//! - [`AsStore`]: the app's [`Documents`] (the person's vault,
//!   `navigator::documents::VaultDocuments` (FastTrackStudios/navigator); the lab's spaces);
//! - [`FileData`]: each extension's `data.json` under the host's data root
//!   (written whole, 0600, one change at a time, never wiped by a file
//!   that does not parse), or in memory without one;
//! - [`HostGate`]: what each loaded extension may reach now (`reads` /
//!   `writes = ["documents"]`, narrowed to what a person granted).
//!
//! A browser's host gets the same client over vox on the org lane, the
//! node serving the same `DocumentsServer`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect::action::Principal;
use architect_documents as svc;
use serde_json::Value;

use crate::capabilities::DOCUMENTS;
use crate::documents::{DocEdit, DocError, DocInfo, Documents};
use crate::host::{Host, WeakHost};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A wire edit as the store's.
#[must_use]
pub fn edit_of(e: &svc::Edit) -> DocEdit {
    let n = |x: u64| usize::try_from(x).unwrap_or(usize::MAX);
    match e {
        svc::Edit::Insert { at, text } => DocEdit::Insert {
            at: n(*at),
            text: text.clone(),
        },
        svc::Edit::Delete { at, len } => DocEdit::Delete {
            at: n(*at),
            len: n(*len),
        },
        svc::Edit::Append { text } => DocEdit::Append { text: text.clone() },
        svc::Edit::Replace { text } => DocEdit::Replace { text: text.clone() },
    }
}

/// A host request's edit as the wire's.
#[must_use]
pub fn wire_edit(e: &DocEdit) -> svc::Edit {
    let n = |x: usize| u64::try_from(x).unwrap_or(u64::MAX);
    match e {
        DocEdit::Insert { at, text } => svc::Edit::Insert {
            at: n(*at),
            text: text.clone(),
        },
        DocEdit::Delete { at, len } => svc::Edit::Delete {
            at: n(*at),
            len: n(*len),
        },
        DocEdit::Append { text } => svc::Edit::Append { text: text.clone() },
        DocEdit::Replace { text } => svc::Edit::Replace { text: text.clone() },
    }
}

fn doc_of(i: DocInfo) -> svc::Doc {
    svc::Doc {
        path: i.path,
        name: i.name,
        space: i.space,
    }
}

/// The wire's document as the host's.
#[must_use]
pub fn info_of(d: svc::Doc) -> DocInfo {
    DocInfo {
        path: d.path,
        name: d.name,
        space: d.space,
    }
}

fn err_of(e: DocError) -> svc::DocsError {
    match e {
        DocError::NotFound(w) => svc::DocsError::NotFound(w),
        DocError::Exists(p) => svc::DocsError::Exists(p),
        DocError::Invalid(w) => svc::DocsError::Invalid(w),
        DocError::Unsupported(w) => svc::DocsError::Unsupported(w),
        DocError::Failed(w) => svc::DocsError::Failed(w),
    }
}

/// The app's [`Documents`] as the service's store.
pub struct AsStore(pub Arc<dyn Documents>);

impl svc::DocumentStore for AsStore {
    fn list(&self, under: Option<&str>) -> Result<Vec<svc::Doc>, svc::DocsError> {
        Ok(self
            .0
            .list(under)
            .map_err(err_of)?
            .into_iter()
            .map(doc_of)
            .collect())
    }

    fn read(&self, path: &str) -> Result<String, svc::DocsError> {
        self.0.read(path).map_err(err_of)
    }

    fn create(&self, path: &str, text: &str, ext: &str) -> Result<svc::Doc, svc::DocsError> {
        self.0
            .create(path, text, &Principal::extension(ext, None))
            .map(doc_of)
            .map_err(err_of)
    }

    fn edit(
        &self,
        path: &str,
        edits: &[svc::Edit],
        step: &str,
        ext: &str,
    ) -> Result<svc::Edited, svc::DocsError> {
        let edits: Vec<DocEdit> = edits.iter().map(edit_of).collect();
        self.0
            .edit(path, &edits, step, &Principal::extension(ext, None))
            .map(|e| svc::Edited {
                text: e.text,
                changed: e.changed,
            })
            .map_err(err_of)
    }

    fn rename(&self, path: &str, to: &str, ext: &str) -> Result<svc::Doc, svc::DocsError> {
        self.0
            .rename(path, to, &Principal::extension(ext, None))
            .map(doc_of)
            .map_err(err_of)
    }

    fn delete(&self, path: &str, ext: &str) -> Result<(), svc::DocsError> {
        self.0
            .delete(path, &Principal::extension(ext, None))
            .map_err(err_of)
    }

    fn metadata(&self, query: &svc::MetaQuery) -> Result<String, svc::DocsError> {
        self.0.metadata(query).map_err(err_of)
    }

    fn undo(&self, path: &str, ext: &str) -> Result<Option<svc::Undone>, svc::DocsError> {
        self.0
            .undo(path, &Principal::extension(ext, None))
            .map(|u| {
                u.map(|u| svc::Undone {
                    step: u.step,
                    text: u.text,
                })
            })
            .map_err(err_of)
    }

    fn redo(&self, path: &str, ext: &str) -> Result<Option<svc::Undone>, svc::DocsError> {
        self.0
            .redo(path, &Principal::extension(ext, None))
            .map(|u| {
                u.map(|u| svc::Undone {
                    step: u.step,
                    text: u.text,
                })
            })
            .map_err(err_of)
    }
}

/// No documents: every call says so.
pub struct NoDocuments;

impl svc::DocumentStore for NoDocuments {
    fn list(&self, _: Option<&str>) -> Result<Vec<svc::Doc>, svc::DocsError> {
        Err(none())
    }
    fn read(&self, _: &str) -> Result<String, svc::DocsError> {
        Err(none())
    }
    fn create(&self, _: &str, _: &str, _: &str) -> Result<svc::Doc, svc::DocsError> {
        Err(none())
    }
    fn edit(
        &self,
        _: &str,
        _: &[svc::Edit],
        _: &str,
        _: &str,
    ) -> Result<svc::Edited, svc::DocsError> {
        Err(none())
    }
    fn rename(&self, _: &str, _: &str, _: &str) -> Result<svc::Doc, svc::DocsError> {
        Err(none())
    }
    fn delete(&self, _: &str, _: &str) -> Result<(), svc::DocsError> {
        Err(none())
    }
}

fn none() -> svc::DocsError {
    svc::DocsError::Unsupported("documents: this app gives extensions none".to_owned())
}

/// What each loaded extension may reach now: its capabilities, narrowed to
/// what a person granted ([`Host::capabilities`]).
pub(crate) struct HostGate(pub(crate) WeakHost);

impl svc::Gate for HostGate {
    fn allows(&self, extension: &str, access: svc::Access) -> bool {
        let Some(host) = self.0.upgrade() else {
            return false;
        };
        let Some(may) = host.capabilities(extension) else {
            return false;
        };
        match access {
            svc::Access::Read => may.may_read(DOCUMENTS),
            svc::Access::Write => may.may_write(DOCUMENTS),
            svc::Access::Own => true,
        }
    }
}

/// Each extension's data: `<root>/<slug>/data.json`, or in memory
/// without a root.
pub struct FileData {
    root: Option<PathBuf>,
    memory: Arc<Mutex<BTreeMap<String, BTreeMap<String, Value>>>>,
    /// With no root: the app's store (a browser's `localStorage`, #495),
    /// before memory.
    store: Option<Arc<dyn crate::hostapi::DataStore>>,
}

impl FileData {
    /// Its data under `root` (none: in `memory`).
    #[must_use]
    pub const fn new(
        root: Option<PathBuf>,
        memory: Arc<Mutex<BTreeMap<String, BTreeMap<String, Value>>>>,
    ) -> Self {
        Self {
            root,
            memory,
            store: None,
        }
    }

    /// With no root, kept in `store` (`Host::with_data_store`) rather
    /// than in memory.
    #[must_use]
    pub fn with_store(mut self, store: Option<Arc<dyn crate::hostapi::DataStore>>) -> Self {
        self.store = store;
        self
    }

    fn file(&self, ext: &str) -> Option<PathBuf> {
        self.root
            .as_ref()
            .map(|r| r.join(crate::install::slug(ext)).join("data.json"))
    }

    /// The lock on `ext`'s data: one read-change-write at a time.
    fn hold(&self, ext: &str) -> std::sync::MutexGuard<'static, ()> {
        static LOCKS: std::sync::LazyLock<Mutex<BTreeMap<String, &'static Mutex<()>>>> =
            std::sync::LazyLock::new(Mutex::default);
        let key = self
            .file(ext)
            .map_or_else(|| format!("mem:{ext}"), |f| f.display().to_string());
        let one: &'static Mutex<()> = lock(&LOCKS)
            .entry(key)
            .or_insert_with(|| Box::leak(Box::new(Mutex::new(()))));
        lock(one)
    }

    /// Its data: none yet is empty; a file that does not parse is an
    /// error, never read as empty, so no write wipes it.
    fn read(&self, ext: &str) -> Result<BTreeMap<String, Value>, svc::DocsError> {
        let Some(file) = self.file(ext) else {
            if let Some(store) = &self.store {
                return Ok(store.load(ext).unwrap_or_default());
            }
            return Ok(lock(&self.memory).get(ext).cloned().unwrap_or_default());
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(svc::DocsError::Failed(format!("{}: {e}", file.display()))),
        };
        serde_json::from_str(&text).map_err(|e| {
            svc::DocsError::Failed(format!(
                "{}: not its data ({e}); left as it is, and nothing is written over it",
                file.display()
            ))
        })
    }

    fn write(&self, ext: &str, data: BTreeMap<String, Value>) -> Result<(), svc::DocsError> {
        let Some(file) = self.file(ext) else {
            if let Some(store) = &self.store {
                return store.keep(ext, &data).map_err(svc::DocsError::Failed);
            }
            lock(&self.memory).insert(ext.to_owned(), data);
            return Ok(());
        };
        let failed =
            |e: &dyn std::fmt::Display| svc::DocsError::Failed(format!("{}: {e}", file.display()));
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| failed(&e))?;
        }
        let text = serde_json::to_string_pretty(&data).map_err(|e| failed(&e))?;
        write_private(&file, text.as_bytes()).map_err(|e| failed(&e))
    }
}

impl svc::DataStore for FileData {
    fn get(&self, extension: &str, key: &str) -> Result<Option<String>, svc::DocsError> {
        let _held = self.hold(extension);
        Ok(self.read(extension)?.remove(key).map(|v| v.to_string()))
    }

    fn set(&self, extension: &str, key: &str, value: Option<&str>) -> Result<(), svc::DocsError> {
        let _held = self.hold(extension);
        let mut data = self.read(extension)?;
        match value {
            Some(v) => {
                let v = serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_owned()));
                data.insert(key.to_owned(), v);
            }
            None => {
                data.remove(key);
            }
        }
        self.write(extension, data)
    }
}

/// `bytes` as the file at `path`, atomically, readable by its owner only.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// The runtime the in-process service runs on: the server's sessions,
/// whichever executor awaits the client. Native only.
#[cfg(not(target_family = "wasm"))]
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: std::sync::OnceLock<Result<tokio::runtime::Runtime, String>> =
        std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("ext-documents")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// A client of `server`, served in-process for as long as `scope` lives:
/// on the service's own runtime natively, with the browser's
/// `spawn_local` in a browser.
///
/// # Errors
///
/// No runtime, or the session could not be made.
pub(crate) async fn establish(
    server: svc::DocumentsServer,
    scope: &Arc<architect::resource::Scope>,
) -> Result<svc::DocumentsClient, String> {
    let local = architect::serve_local(server, scope);
    #[cfg(not(target_family = "wasm"))]
    {
        runtime()?
            .spawn(async move { local.establish::<svc::DocumentsClient>().await })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(target_family = "wasm")]
    {
        local
            .establish::<svc::DocumentsClient>()
            .await
            .map_err(|e| e.to_string())
    }
}

impl Host {
    /// The documents service's client: made the first time, again after
    /// the documents or the data root changed.
    pub(crate) async fn documents_client(
        &self,
    ) -> Result<svc::DocumentsClient, crate::hostcall::HostError> {
        if let Some(c) = self.cached_documents_client() {
            return Ok(c);
        }
        let docs: Arc<dyn svc::DocumentStore> = match self.documents() {
            Some(d) => Arc::new(AsStore(d)),
            None => Arc::new(NoDocuments),
        };
        let data =
            FileData::new(self.data_root(), self.memory_data()).with_store(self.data_store());
        let server = svc::DocumentsServer::from_arcs(
            docs,
            Arc::new(data),
            Arc::new(HostGate(self.downgrade())),
        );
        let scope = architect::resource::Scope::new();
        let client = establish(server, &scope)
            .await
            .map_err(|message| crate::hostcall::HostError::Failed { message })?;
        self.keep_documents_client(client.clone(), scope);
        Ok(client)
    }
}

/// A service error as the host's.
#[must_use]
pub fn host_error(e: architect::vox::VoxError<svc::DocsError>) -> crate::hostcall::HostError {
    use crate::hostcall::HostError;
    match e {
        architect::vox::VoxError::User(e) => match *e {
            svc::DocsError::Refused {
                extension,
                capability,
            } => HostError::Refused {
                why: format!("{extension} may not: it needs {capability}"),
                capability,
            },
            svc::DocsError::NotFound(what) => HostError::NotFound { what },
            svc::DocsError::Unsupported(what) => HostError::Unsupported { what },
            other => HostError::Failed {
                message: other.to_string(),
            },
        },
        other => HostError::Failed {
            message: format!("the documents service: {other:?}"),
        },
    }
}
