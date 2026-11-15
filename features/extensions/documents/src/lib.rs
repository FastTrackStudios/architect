//! The documents service: how extensions reach the app's documents, their
//! own data and the vault's per-note metadata (#706, #710).
//!
//! **One service, one implementation, every transport** ("inject the
//! transport, not the backend", `crates/architect/src/local.rs`):
//!
//! - [`Documents`] is the `#[architect::rpc]` trait. The extension
//!   host calls it only through the generated [`DocumentsClient`]:
//!   in-process on the desk and iOS (`architect::serve_local`), over vox
//!   on the org lane in a browser (the node serving the same
//!   implementation), and in-process in wasm for the demo.
//! - [`DocumentsServer`] is its one implementation. The gate is in it:
//!   every call names the extension it is for, and [`Gate`] says whether
//!   that extension may read or change the documents
//!   (`reads`/`writes = ["documents"]`), so every transport gets the same
//!   check. Below the gate it calls a [`DocumentStore`] (the vault:
//!   `architect_navigator::documents::VaultDocuments`; an in-memory one in
//!   tests and the demo, [`MemoryStore`]) and a [`DataStore`] (each
//!   extension's own data).
//!
//! **Wasm-clean.** This crate is: the trait, the wire types, the gate and
//! [`MemoryStore`]. What is not is below it, behind [`DocumentStore`] and
//! [`DataStore`]: the vault on disk (`vault-live`'s file system and
//! watcher, `VaultSource`) and the data files (`std::fs`), which the
//! native app and the node implement; a browser's demo uses the memory
//! stores.
//!
//! **Who is asking.** The `extension` argument is the extension the host
//! runs the call for. The host is the client (the app's, or a browser's
//! web host on the person's own lane), never the extension's code, so it
//! is trusted to name it; the node checks it against the grants the
//! person gave on that lane ([`Gate`]).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

/// One of the app's documents, as an extension sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq, facet::Facet)]
pub struct Doc {
    /// Its path: `notes/Inbox.md`.
    pub path: String,
    /// Its name, the path's last part: `Inbox.md`.
    pub name: String,
    /// The space it is in: its path's first folder (none at the top).
    pub space: Option<String>,
}

impl Doc {
    /// The document at `path`.
    #[must_use]
    pub fn of(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            space: path.split_once('/').map(|(s, _)| s.to_owned()),
        }
    }
}

/// One edit of a document's text, in a transaction. Positions count
/// characters (Unicode scalar values).
#[derive(Clone, Debug, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum Edit {
    /// `text` at character `at` (past the end: at the end).
    Insert { at: u64, text: String },
    /// `len` characters from character `at`.
    Delete { at: u64, len: u64 },
    /// `text` at the end.
    Append { text: String },
    /// The whole text is `text`, as the edits that differ.
    Replace { text: String },
}

/// A `VAULT_METADATA` query: `path` for one note, or a page of `folder`'s
/// (`""`: the whole vault), at most `limit` after `cursor`.
#[derive(Clone, Debug, Default, PartialEq, Eq, facet::Facet)]
pub struct MetaQuery {
    pub path: Option<String>,
    pub folder: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// What an edit did: the text after, and whether it changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, facet::Facet)]
pub struct Edited {
    pub text: String,
    pub changed: bool,
}

/// Why the documents service did not do it.
#[derive(Clone, Debug, PartialEq, Eq, facet::Facet, thiserror::Error)]
#[repr(u8)]
pub enum DocsError {
    /// The extension may not: `capability` is what it lacks
    /// (`reads.documents`, `writes.documents`).
    #[error("{extension} may not: it needs {capability}")]
    Refused {
        extension: String,
        capability: String,
    },
    /// No document (or space) of that path.
    #[error("no {0}")]
    NotFound(String),
    /// A document is there already.
    #[error("{0} exists already")]
    Exists(String),
    /// Not a path, or an edit outside the text.
    #[error("{0}")]
    Invalid(String),
    /// The app gives no documents (or no data, or no metadata).
    #[error("{0}")]
    Unsupported(String),
    /// It was allowed and did not work.
    #[error("{0}")]
    Failed(String),
}

/// What a call needs of an extension's grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// List, read, watch, metadata: `reads = ["documents"]`.
    Read,
    /// Create, edit, rename, move, delete: `writes = ["documents"]`.
    Write,
    /// Its own data: every extension has its own.
    Own,
}

impl Access {
    /// The capability it names, for a refusal.
    #[must_use]
    pub const fn capability(self) -> &'static str {
        match self {
            Self::Read => "reads.documents",
            Self::Write => "writes.documents",
            Self::Own => "data",
        }
    }
}

/// Whether an extension may: what the host (or the node, for its lane)
/// knows of the grants a person gave.
pub trait Gate: Send + Sync {
    /// Whether `extension` may make a call needing `access`.
    fn allows(&self, extension: &str, access: Access) -> bool;
}

/// A gate that allows everything: a test's, or a first-party tool's.
#[derive(Clone, Copy, Debug, Default)]
pub struct Open;

impl Gate for Open {
    fn allows(&self, _: &str, _: Access) -> bool {
        true
    }
}

/// The documents under the gate: the vault (or an app's own set). Its
/// paths are checked (relative, `/`-separated, no `..`) before it is
/// called.
pub trait DocumentStore: Send + Sync {
    /// Every document, or those under folder `under`.
    ///
    /// # Errors
    ///
    /// It could not list them.
    fn list(&self, under: Option<&str>) -> Result<Vec<Doc>, DocsError>;
    /// Document `path`'s text.
    ///
    /// # Errors
    ///
    /// [`DocsError::NotFound`].
    fn read(&self, path: &str) -> Result<String, DocsError>;
    /// A new document at `path` holding `text`, made as `extension`.
    ///
    /// # Errors
    ///
    /// [`DocsError::Exists`], or a path it does not keep.
    fn create(&self, path: &str, text: &str, extension: &str) -> Result<Doc, DocsError>;
    /// `edits`, in order, as one transaction named `step`, made as
    /// `extension`.
    ///
    /// # Errors
    ///
    /// No document, or an edit outside its text (nothing changed).
    fn edit(
        &self,
        path: &str,
        edits: &[Edit],
        step: &str,
        extension: &str,
    ) -> Result<Edited, DocsError>;
    /// Document `path` at `to`.
    ///
    /// # Errors
    ///
    /// No document at `path`, or one at `to`.
    fn rename(&self, path: &str, to: &str, extension: &str) -> Result<Doc, DocsError>;
    /// Deletes document `path`.
    ///
    /// # Errors
    ///
    /// No document at `path`.
    fn delete(&self, path: &str, extension: &str) -> Result<(), DocsError>;
    /// The vault's parsed metadata as `VAULT_METADATA` answers it (JSON):
    /// one note's, or a page of a folder's.
    ///
    /// # Errors
    ///
    /// No note, neither given, or no metadata here.
    fn metadata(&self, query: &MetaQuery) -> Result<String, DocsError> {
        let _ = query;
        Err(DocsError::Unsupported(
            "metadata: these documents have none".to_owned(),
        ))
    }
}

/// Each extension's own data: keys to JSON texts.
pub trait DataStore: Send + Sync {
    /// `extension`'s value of `key`, as JSON.
    ///
    /// # Errors
    ///
    /// Its data cannot be read (never read as empty).
    fn get(&self, extension: &str, key: &str) -> Result<Option<String>, DocsError>;
    /// Sets (or, `None`, removes) `extension`'s `key`.
    ///
    /// # Errors
    ///
    /// Its data cannot be read or written.
    fn set(&self, extension: &str, key: &str, value: Option<&str>) -> Result<(), DocsError>;
}

/// `path` checked: relative, `/`-separated, no empty part, `.` or `..`.
///
/// # Errors
///
/// [`DocsError::Invalid`].
pub fn checked(path: &str) -> Result<&str, DocsError> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..");
    if bad {
        Err(DocsError::Invalid(format!(
            "{path:?} is not a document's path (relative, `/`-separated, no `..`)"
        )))
    } else {
        Ok(path)
    }
}

/// The documents service: what an extension host asks for an extension.
/// Every method names the extension it is for.
#[architect::rpc]
pub trait Documents {
    /// Every document, or those under folder `under` (`reads`).
    fn list(&self, extension: String, under: Option<String>) -> Result<Vec<Doc>, DocsError>;
    /// A document's text (`reads`).
    fn read(&self, extension: String, path: String) -> Result<String, DocsError>;
    /// A new document (`writes`).
    fn create(&self, extension: String, path: String, text: String) -> Result<Doc, DocsError>;
    /// Edits as one transaction named `step` (`writes`): the text after,
    /// and whether it changed.
    fn edit(
        &self,
        extension: String,
        path: String,
        edits: Vec<Edit>,
        step: String,
    ) -> Result<Edited, DocsError>;
    /// Renames or moves a document (`writes`).
    fn rename(&self, extension: String, path: String, to: String) -> Result<Doc, DocsError>;
    /// Deletes a document (`writes`).
    fn delete(&self, extension: String, path: String) -> Result<(), DocsError>;
    /// The vault's per-note metadata, `VAULT_METADATA`'s answer as JSON
    /// (`reads`): `path` for one note, or a page of `folder`'s.
    fn metadata(&self, extension: String, query: MetaQuery) -> Result<String, DocsError>;
    /// The extension's own value of `key`, as JSON.
    fn data(&self, extension: String, key: String) -> Result<Option<String>, DocsError>;
    /// Sets (or, `None`, removes) the extension's own `key`.
    fn set_data(
        &self,
        extension: String,
        key: String,
        value: Option<String>,
    ) -> Result<(), DocsError>;
}

/// The one implementation of [`Documents`]: the gate, then the
/// stores. Cheap to clone.
#[derive(Clone, architect::HasDispatcher)]
pub struct DocumentsServer {
    docs: Arc<dyn DocumentStore>,
    data: Arc<dyn DataStore>,
    gate: Arc<dyn Gate>,
}

impl DocumentsServer {
    /// The service over `docs` and `data`, gated by `gate`.
    pub fn new(
        docs: impl DocumentStore + 'static,
        data: impl DataStore + 'static,
        gate: impl Gate + 'static,
    ) -> Self {
        Self {
            docs: Arc::new(docs),
            data: Arc::new(data),
            gate: Arc::new(gate),
        }
    }

    /// The service over stores already shared.
    #[must_use]
    pub fn from_arcs(
        docs: Arc<dyn DocumentStore>,
        data: Arc<dyn DataStore>,
        gate: Arc<dyn Gate>,
    ) -> Self {
        Self { docs, data, gate }
    }

    fn may(&self, extension: &str, access: Access) -> Result<(), DocsError> {
        if self.gate.allows(extension, access) {
            Ok(())
        } else {
            Err(DocsError::Refused {
                extension: extension.to_owned(),
                capability: access.capability().to_owned(),
            })
        }
    }
}

impl Documents for DocumentsServer {
    fn list(&self, extension: String, under: Option<String>) -> Result<Vec<Doc>, DocsError> {
        self.may(&extension, Access::Read)?;
        self.docs.list(under.as_deref().filter(|u| !u.is_empty()))
    }

    fn read(&self, extension: String, path: String) -> Result<String, DocsError> {
        self.may(&extension, Access::Read)?;
        self.docs.read(checked(&path)?)
    }

    fn create(&self, extension: String, path: String, text: String) -> Result<Doc, DocsError> {
        self.may(&extension, Access::Write)?;
        self.docs.create(checked(&path)?, &text, &extension)
    }

    fn edit(
        &self,
        extension: String,
        path: String,
        edits: Vec<Edit>,
        step: String,
    ) -> Result<Edited, DocsError> {
        self.may(&extension, Access::Write)?;
        self.docs.edit(checked(&path)?, &edits, &step, &extension)
    }

    fn rename(&self, extension: String, path: String, to: String) -> Result<Doc, DocsError> {
        self.may(&extension, Access::Write)?;
        self.docs.rename(checked(&path)?, checked(&to)?, &extension)
    }

    fn delete(&self, extension: String, path: String) -> Result<(), DocsError> {
        self.may(&extension, Access::Write)?;
        self.docs.delete(checked(&path)?, &extension)
    }

    fn metadata(&self, extension: String, query: MetaQuery) -> Result<String, DocsError> {
        self.may(&extension, Access::Read)?;
        if let Some(p) = &query.path {
            checked(p)?;
        }
        self.docs.metadata(&query)
    }

    fn data(&self, extension: String, key: String) -> Result<Option<String>, DocsError> {
        self.may(&extension, Access::Own)?;
        self.data.get(&extension, &key)
    }

    fn set_data(
        &self,
        extension: String,
        key: String,
        value: Option<String>,
    ) -> Result<(), DocsError> {
        self.may(&extension, Access::Own)?;
        self.data.set(&extension, &key, value.as_deref())
    }
}

#[cfg(feature = "vox")]
impl architect::Services for DocumentsServer {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![prelude::DocumentsService]
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Documents and data in memory: tests, and the demo in a browser.
#[derive(Clone, Default)]
pub struct MemoryStore {
    texts: Arc<Mutex<BTreeMap<String, String>>>,
    data: Arc<Mutex<BTreeMap<(String, String), String>>>,
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The store holding `path` with `text`.
    #[must_use]
    pub fn with(self, path: &str, text: &str) -> Self {
        lock(&self.texts).insert(path.to_owned(), text.to_owned());
        self
    }
}

/// `edits` applied to `text` (characters, in order).
///
/// # Errors
///
/// An edit outside the text.
pub fn apply(text: &str, edits: &[Edit]) -> Result<String, DocsError> {
    let mut chars: Vec<char> = text.chars().collect();
    let at_of = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
    for (i, edit) in edits.iter().enumerate() {
        match edit {
            Edit::Insert { at, text } => {
                let at = at_of(*at).min(chars.len());
                chars.splice(at..at, text.chars());
            }
            Edit::Append { text } => chars.extend(text.chars()),
            Edit::Delete { at, len } => {
                let (at, len) = (at_of(*at), at_of(*len));
                let end = at
                    .checked_add(len)
                    .filter(|e| *e <= chars.len())
                    .ok_or_else(|| {
                        DocsError::Invalid(format!(
                            "edit {i}: deletes {len} from {at}, past the text's end ({})",
                            chars.len()
                        ))
                    })?;
                chars.drain(at..end);
            }
            Edit::Replace { text } => chars = text.chars().collect(),
        }
    }
    Ok(chars.into_iter().collect())
}

impl DocumentStore for MemoryStore {
    fn list(&self, under: Option<&str>) -> Result<Vec<Doc>, DocsError> {
        let prefix = under.map(|u| format!("{}/", u.trim_end_matches('/')));
        Ok(lock(&self.texts)
            .keys()
            .filter(|p| prefix.as_deref().is_none_or(|u| p.starts_with(u)))
            .map(|p| Doc::of(p))
            .collect())
    }

    fn read(&self, path: &str) -> Result<String, DocsError> {
        lock(&self.texts)
            .get(path)
            .cloned()
            .ok_or_else(|| DocsError::NotFound(format!("document {path}")))
    }

    fn create(&self, path: &str, text: &str, _: &str) -> Result<Doc, DocsError> {
        let mut texts = lock(&self.texts);
        if texts.contains_key(path) {
            return Err(DocsError::Exists(path.to_owned()));
        }
        texts.insert(path.to_owned(), text.to_owned());
        drop(texts);
        Ok(Doc::of(path))
    }

    fn edit(&self, path: &str, edits: &[Edit], _: &str, _: &str) -> Result<Edited, DocsError> {
        let mut texts = lock(&self.texts);
        let text = texts
            .get_mut(path)
            .ok_or_else(|| DocsError::NotFound(format!("document {path}")))?;
        let after = apply(text, edits)?;
        let changed = *text != after;
        text.clone_from(&after);
        drop(texts);
        Ok(Edited {
            text: after,
            changed,
        })
    }

    fn rename(&self, path: &str, to: &str, _: &str) -> Result<Doc, DocsError> {
        let mut texts = lock(&self.texts);
        if texts.contains_key(to) {
            return Err(DocsError::Exists(to.to_owned()));
        }
        let text = texts
            .remove(path)
            .ok_or_else(|| DocsError::NotFound(format!("document {path}")))?;
        texts.insert(to.to_owned(), text);
        drop(texts);
        Ok(Doc::of(to))
    }

    fn delete(&self, path: &str, _: &str) -> Result<(), DocsError> {
        lock(&self.texts)
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| DocsError::NotFound(format!("document {path}")))
    }
}

impl DataStore for MemoryStore {
    fn get(&self, extension: &str, key: &str) -> Result<Option<String>, DocsError> {
        Ok(lock(&self.data)
            .get(&(extension.to_owned(), key.to_owned()))
            .cloned())
    }

    fn set(&self, extension: &str, key: &str, value: Option<&str>) -> Result<(), DocsError> {
        let k = (extension.to_owned(), key.to_owned());
        let mut data = lock(&self.data);
        match value {
            Some(v) => {
                data.insert(k, v.to_owned());
            }
            None => {
                data.remove(&k);
            }
        }
        drop(data);
        Ok(())
    }
}
