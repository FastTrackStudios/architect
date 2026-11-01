//! A document's file (`docs/content/architecture/documents.md`, "The
//! file"): one file per document, holding its branches, every finished
//! step, its snapshots, its saved versions and its small blobs.
//!
//! Studio's `.session` store (`studio:crates/daw/session-format/src/store/mod.rs`)
//! generalised. The same five things, kept as records in one append-only
//! file rather than tables in a database, so the file is crash-safe by
//! its framing and a browser reads it from bytes:
//!
//! ```text
//! meta       key → value              the kind, its schema stamp, the id
//! blobs      sha256 → kind, bytes     kept once, whichever branch uses them
//! branches   name → parent, base      `main`, and the branches forked from it
//! steps      seq → branch, name, at,  each finished undo step's Loro update;
//!            frontiers, update        its name and place kept once folded
//! snapshots  branch → upto, snapshot  a branch folded as of step `upto`
//! ```
//!
//! and checkpoints — a named point on a branch, which is what "Save" makes.
//! The bytes are [`format`]'s; where they live is a [`Storage`] (a file on
//! disk, or bytes in memory).
//!
//! - **No unsaved state.** [`Branch::transact`] runs one named step and
//!   returns once the step is durable; [`Branch::undo`] and
//!   [`Branch::redo`] too. Killed at any moment, the file reopens with
//!   every step that finished. "Save" ([`Branch::save`]) names a version
//!   and folds the branch's steps into a snapshot.
//! - **Recovery.** A write the crash cut short is a torn record at the
//!   end; opening the file drops it ([`DocFile::recovered`] says how many
//!   bytes) and replays the steps since the last snapshot.
//! - **Branches and versions.** [`Branch::fork`], [`DocFile::open_branch`]
//!   (any number side by side), [`Branch::merge`], [`DocFile::delete_branch`];
//!   [`DocFile::versions`] lists a branch's steps and checkpoints,
//!   [`DocFile::open_version`] opens one read-only, [`DocFile::restore`]
//!   makes one a new branch.
//! - **The kind.** A file is made for a [`DocKind`] and says so in `meta`;
//!   opening a branch brings it up to the kind's schema, the migration a
//!   step of its own.
//!
//! What lives beside the file and not in it — large media, fetched by
//! hash — is the blob layer's.

mod branch;
mod format;
mod index;
mod persistence;
mod storage;

use std::sync::Arc;

use loro::{Frontiers, LoroDoc};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

pub use self::branch::Branch;
pub use self::format::FORMAT;
use self::format::Record;
use self::index::{Index, loro};
pub use self::persistence::BranchPersistence;
#[cfg(not(target_arch = "wasm32"))]
pub use self::storage::DiskStorage;
pub use self::storage::{MemoryStorage, Storage};
use crate::kinds::{DocKind, DocKinds, stamp_of};

/// The branch every document starts with.
pub const MAIN: &str = "main";

/// How many named steps a branch keeps to undo.
pub const UNDO_STEPS: usize = 200;

/// What went wrong with a document's file.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("storage: {0}")]
    Storage(String),
    #[error("not a document file: {0}")]
    NotADocument(String),
    #[error("loro: {0}")]
    Loro(String),
    #[error("this is a {found} file, not a {expected}")]
    WrongKind { found: String, expected: String },
    #[error("no kind registered for {0} files")]
    UnknownKind(String),
    #[error("schema: {0}")]
    Schema(String),
    #[error("no branch {0}")]
    NoBranch(String),
    #[error("there is already a branch {0}")]
    BranchExists(String),
    #[error("main is the document itself and cannot be deleted")]
    DeleteMain,
    #[error("blob {0} does not hash to its name")]
    CorruptBlob(String),
}

/// A branch, as [`DocFile::branches`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    /// The branch it was forked from; `None` for `main` (and for a branch
    /// whose parent was deleted after it).
    pub parent: Option<String>,
    /// When it was made, in milliseconds since the epoch.
    pub at: i64,
}

/// A blob in the file, as [`DocFile::blobs`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInfo {
    /// Its SHA-256, lowercase hex.
    pub hash: String,
    /// What the app said it is (`fx`, `midi`, `image`).
    pub kind: String,
    pub size: u64,
}

/// What a version is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionKind {
    /// A finished step ("Split item").
    Step,
    /// A named point ("Save" — "Mix sent to Dan").
    Checkpoint,
}

/// A point on a branch that can be opened or restored: a step or a
/// checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub branch: String,
    pub name: String,
    /// In milliseconds since the epoch.
    pub at: i64,
    pub kind: VersionKind,
    /// The branch's Loro frontiers at this point.
    pub frontiers: Frontiers,
}

/// What opening a file found cut short by a crash, and dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    /// The bytes past the last whole record: a write that never finished.
    pub dropped_bytes: u64,
}

/// A blob's name: its SHA-256, lowercase hex (the name `architect::blob`
/// and Studio's store give it).
#[must_use]
pub fn hash_of(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Milliseconds since the epoch — when a step, branch or version was made.
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// One document's file, open. Cheap to clone; clones share it.
#[derive(Clone)]
pub struct DocFile {
    inner: Arc<FileInner>,
}

struct FileInner {
    storage: Box<dyn Storage>,
    /// Held across a write to storage, so the file's records and the
    /// index are in the same order.
    index: AsyncMutex<Index>,
    kind: DocKind,
    recovered: Option<Recovery>,
}

impl std::fmt::Debug for DocFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocFile")
            .field("kind", &self.inner.kind.id)
            .finish_non_exhaustive()
    }
}

impl DocFile {
    /// Makes a new document of `kind` in `storage` (whatever was there is
    /// replaced): its `meta`, and `main` stamped with the kind's schema.
    pub async fn create(storage: impl Storage, kind: &DocKind) -> Result<Self, FileError> {
        Self::create_as(storage, kind, Uuid::new_v4()).await
    }

    /// [`Self::create`], for the document `id` (the id a catalogue knows
    /// it by: a document made on this device and registered later).
    pub async fn create_as(
        storage: impl Storage,
        kind: &DocKind,
        id: Uuid,
    ) -> Result<Self, FileError> {
        let file = Self::empty(storage, kind, id, Some(kind.stamp)).await?;
        // Stamped at this build's schema, not upgraded to it: a new
        // document has nothing to migrate.
        let mut main = Branch::new(file.clone(), MAIN.to_owned(), LoroDoc::new());
        main.stamp(kind).await?;
        Ok(file)
    }

    /// A file of `kind` for the document `id` made somewhere else, its
    /// `main` empty (whatever was in `storage` is replaced). Nothing is
    /// written into it, not even the kind's stamp: its history, stamp
    /// included, arrives from the device that made it, and an edit of
    /// this file's own before that would be a history the document's
    /// host refuses as another lineage.
    pub async fn adopt(storage: impl Storage, kind: &DocKind, id: Uuid) -> Result<Self, FileError> {
        Self::empty(storage, kind, id, None).await
    }

    /// A file holding only its `meta` and an empty `main`.
    async fn empty(
        storage: impl Storage,
        kind: &DocKind,
        id: Uuid,
        stamp: Option<u32>,
    ) -> Result<Self, FileError> {
        let mut records = vec![meta("kind", &kind.id)];
        if let Some(stamp) = stamp {
            records.push(meta("schema", &stamp.to_string()));
        }
        records.extend([
            meta("id", &id.to_string()),
            Record::Branch {
                name: MAIN.to_owned(),
                parent: None,
                base: None,
                at: now(),
            },
        ]);
        storage.replace(&image(&records)?).await?;
        Ok(Self::from_parts(
            Box::new(storage),
            Index::from_records(records),
            kind,
            None,
        ))
    }

    /// Opens the document of `kind` in `storage`, dropping a torn tail a
    /// crash left (see [`Self::recovered`]).
    pub async fn open(storage: impl Storage, kind: &DocKind) -> Result<Self, FileError> {
        let (index, recovered) = read(&storage).await?;
        let found = index.meta("kind").unwrap_or_default();
        if found != kind.id {
            return Err(FileError::WrongKind {
                found: found.to_owned(),
                expected: kind.id.clone(),
            });
        }
        check_schema(&index, kind)?;
        Ok(Self::from_parts(Box::new(storage), index, kind, recovered))
    }

    /// Opens the document in `storage` as whichever of `kinds` it says it is.
    pub async fn open_any(storage: impl Storage, kinds: &DocKinds) -> Result<Self, FileError> {
        let (index, recovered) = read(&storage).await?;
        let found = index.meta("kind").unwrap_or_default();
        let kind = kinds
            .get(found)
            .ok_or_else(|| FileError::UnknownKind(found.to_owned()))?;
        check_schema(&index, kind)?;
        Ok(Self::from_parts(Box::new(storage), index, kind, recovered))
    }

    /// The document `bytes` are (a file the browser was handed), in memory.
    pub async fn from_bytes(bytes: Vec<u8>, kind: &DocKind) -> Result<Self, FileError> {
        Self::open(MemoryStorage::from_bytes(bytes), kind).await
    }

    fn from_parts(
        storage: Box<dyn Storage>,
        index: Index,
        kind: &DocKind,
        recovered: Option<Recovery>,
    ) -> Self {
        Self {
            inner: Arc::new(FileInner {
                storage,
                index: AsyncMutex::new(index),
                kind: kind.clone(),
                recovered,
            }),
        }
    }

    #[must_use]
    pub fn kind(&self) -> &DocKind {
        &self.inner.kind
    }

    /// What opening it dropped after a crash; `None` if it was whole.
    #[must_use]
    pub fn recovered(&self) -> Option<Recovery> {
        self.inner.recovered
    }

    /// The document's id (made with it, kept in `meta`).
    pub async fn id(&self) -> Option<Uuid> {
        self.meta("id").await.and_then(|s| s.parse().ok())
    }

    /// A `meta` value.
    pub async fn meta(&self, key: &str) -> Option<String> {
        self.inner.index.lock().await.meta(key).map(str::to_owned)
    }

    /// Sets a `meta` value. `kind`, `schema` and `id` are the file's own.
    pub async fn set_meta(&self, key: &str, value: &str) -> Result<(), FileError> {
        self.write(vec![meta(key, value)]).await
    }

    /// The whole file as bytes, folded to what it holds: one file to send.
    pub async fn to_bytes(&self) -> Result<Vec<u8>, FileError> {
        image(&self.inner.index.lock().await.records())
    }

    // ── Blobs ─────────────────────────────────────────────────────────

    /// Keeps `bytes` (of `kind`: `fx`, `midi`, …) and returns its name.
    /// Bytes already here are not written again.
    pub async fn put_blob(&self, kind: &str, bytes: &[u8]) -> Result<String, FileError> {
        let hash = hash_of(bytes);
        let mut index = self.inner.index.lock().await;
        if !index.has_blob(&hash) {
            let record = Record::Blob {
                hash: hash.clone(),
                kind: kind.to_owned(),
                data: bytes.to_vec(),
            };
            self.inner.storage.append(&format::frame(&record)?).await?;
            index.apply(record);
        }
        drop(index);
        Ok(hash)
    }

    /// The blob named `hash`, checked against its name; `None` if the file
    /// does not hold it.
    pub async fn blob(&self, hash: &str) -> Result<Option<Vec<u8>>, FileError> {
        let bytes = self.inner.index.lock().await.blob(hash).map(<[u8]>::to_vec);
        match bytes {
            Some(b) if hash_of(&b) != hash => Err(FileError::CorruptBlob(hash.to_owned())),
            other => Ok(other),
        }
    }

    pub async fn blobs(&self) -> Vec<BlobInfo> {
        self.inner.index.lock().await.blobs()
    }

    // ── Branches and versions ─────────────────────────────────────────

    /// Every branch: `main` first, then the others oldest first.
    pub async fn branches(&self) -> Vec<BranchInfo> {
        self.inner.index.lock().await.branches()
    }

    /// Opens `name` to read and edit, at the kind's schema (a migration
    /// is a step of its own). Any number of branches — or the same one
    /// twice — open side by side.
    pub async fn open_branch(&self, name: &str) -> Result<Branch, FileError> {
        let doc = self.inner.index.lock().await.materialize(name)?;
        let mut branch = Branch::new(self.clone(), name.to_owned(), doc);
        branch.upgrade(&self.inner.kind).await?;
        Ok(branch)
    }

    /// Forgets `name` and what it kept. A branch forked from it is folded
    /// first, so it stands on its own.
    // The index stays locked across the write, so the file's records and
    // the index are in the same order.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn delete_branch(&self, name: &str) -> Result<(), FileError> {
        if name == MAIN {
            return Err(FileError::DeleteMain);
        }
        let mut index = self.inner.index.lock().await;
        if !index.has_branch(name) {
            return Err(FileError::NoBranch(name.to_owned()));
        }
        let parent = index.parent(name);
        let mut records = Vec::new();
        for child in index.children(name) {
            let doc = index.materialize(&child)?;
            records.push(Record::Snapshot {
                branch: child.clone(),
                upto: index.last_step(&child),
                at: now(),
                data: doc.export(loro::ExportMode::Snapshot).map_err(loro)?,
            });
            records.push(Record::Branch {
                name: child,
                parent: parent.clone(),
                base: None,
                at: now(),
            });
        }
        records.push(Record::DropBranch {
            name: name.to_owned(),
        });
        self.append_locked(&mut index, records).await
    }

    /// `branch`'s finished steps and checkpoints, in the order they were made.
    pub async fn versions(&self, branch: &str) -> Result<Vec<Version>, FileError> {
        let index = self.inner.index.lock().await;
        if !index.has_branch(branch) {
            return Err(FileError::NoBranch(branch.to_owned()));
        }
        index.versions(branch)
    }

    /// The document as it was at `version`, read-only (checked out: Loro
    /// refuses edits to it).
    pub async fn open_version(&self, version: &Version) -> Result<LoroDoc, FileError> {
        let doc = self.inner.index.lock().await.materialize(&version.branch)?;
        doc.checkout(&version.frontiers).map_err(loro)?;
        Ok(doc)
    }

    /// Makes `version` a new branch `name`, forked from its branch there.
    pub async fn restore(&self, version: &Version, name: &str) -> Result<Branch, FileError> {
        self.new_branch(name, &version.branch, &version.frontiers)
            .await?;
        self.open_branch(name).await
    }

    /// Deletes the checkpoint `name` on `branch`.
    pub async fn delete_checkpoint(&self, branch: &str, name: &str) -> Result<(), FileError> {
        self.write(vec![Record::DropCheckpoint {
            branch: branch.to_owned(),
            name: name.to_owned(),
        }])
        .await
    }

    /// A [`Persistence`](crate::Persistence) over `branch`, so a
    /// [`CrdtDoc`](crate::CrdtDoc) — and the sync built on one — keeps
    /// the branch in this file.
    #[must_use]
    pub fn persistence(&self, branch: &str) -> BranchPersistence {
        BranchPersistence::new(self.clone(), branch.to_owned())
    }

    // ── What branches write ───────────────────────────────────────────

    // The index stays locked across the write, so the file's records and
    // the index are in the same order.
    #[allow(clippy::significant_drop_tightening)]
    async fn new_branch(&self, name: &str, parent: &str, at: &Frontiers) -> Result<(), FileError> {
        let mut index = self.inner.index.lock().await;
        if index.has_branch(name) {
            return Err(FileError::BranchExists(name.to_owned()));
        }
        if !index.has_branch(parent) {
            return Err(FileError::NoBranch(parent.to_owned()));
        }
        let record = Record::Branch {
            name: name.to_owned(),
            parent: Some(parent.to_owned()),
            base: Some(at.encode()),
            at: now(),
        };
        self.append_locked(&mut index, vec![record]).await
    }

    /// Appends one finished step to `branch`.
    // The index stays locked across the write, so the file's records and
    // the index are in the same order.
    #[allow(clippy::significant_drop_tightening)]
    async fn append_step(
        &self,
        branch: &str,
        label: &str,
        frontiers: &Frontiers,
        update: Vec<u8>,
    ) -> Result<(), FileError> {
        let mut index = self.inner.index.lock().await;
        if !index.has_branch(branch) {
            return Err(FileError::NoBranch(branch.to_owned()));
        }
        let record = Record::Step {
            seq: index.next_seq(),
            branch: branch.to_owned(),
            label: label.to_owned(),
            at: now(),
            // None written: not a version to open (a raw sync update).
            frontiers: if frontiers.is_empty() {
                Vec::new()
            } else {
                frontiers.encode()
            },
            data: update,
        };
        self.append_locked(&mut index, vec![record]).await
    }

    // The index stays locked across the write, so the file's records and
    // the index are in the same order.
    #[allow(clippy::significant_drop_tightening)]
    async fn checkpoint(
        &self,
        branch: &str,
        name: &str,
        frontiers: &Frontiers,
    ) -> Result<Version, FileError> {
        let mut index = self.inner.index.lock().await;
        if !index.has_branch(branch) {
            return Err(FileError::NoBranch(branch.to_owned()));
        }
        let at = now();
        let record = Record::Checkpoint {
            seq: index.next_seq(),
            branch: branch.to_owned(),
            name: name.to_owned(),
            at,
            frontiers: frontiers.encode(),
        };
        self.append_locked(&mut index, vec![record]).await?;
        Ok(Version {
            branch: branch.to_owned(),
            name: name.to_owned(),
            at,
            kind: VersionKind::Checkpoint,
            frontiers: frontiers.clone(),
        })
    }

    /// Folds `branch`: its steps so far into one snapshot, built from the
    /// file itself (so a step another handle wrote is never lost), and
    /// the file rewritten to what it holds.
    async fn fold(&self, branch: &str) -> Result<(), FileError> {
        let mut index = self.inner.index.lock().await;
        let doc = index.materialize(branch)?;
        let mut folded = index.clone();
        folded.apply(Record::Snapshot {
            branch: branch.to_owned(),
            upto: index.last_step(branch),
            at: now(),
            data: doc.export(loro::ExportMode::Snapshot).map_err(loro)?,
        });
        self.inner
            .storage
            .replace(&image(&folded.records())?)
            .await?;
        *index = folded;
        drop(index);
        Ok(())
    }

    /// `branch` as the file holds it now.
    async fn materialize(&self, branch: &str) -> Result<LoroDoc, FileError> {
        self.inner.index.lock().await.materialize(branch)
    }

    async fn write(&self, records: Vec<Record>) -> Result<(), FileError> {
        let mut index = self.inner.index.lock().await;
        self.append_locked(&mut index, records).await
    }

    /// Writes `records` as one append (durable together, or the torn
    /// tail of a crash), then adds them to the index.
    async fn append_locked(
        &self,
        index: &mut Index,
        records: Vec<Record>,
    ) -> Result<(), FileError> {
        let mut bytes = Vec::new();
        for r in &records {
            bytes.extend(format::frame(r)?);
        }
        self.inner.storage.append(&bytes).await?;
        for r in records {
            index.apply(r);
        }
        Ok(())
    }

    /// Records `main`'s schema stamp in `meta`.
    async fn set_schema(&self, doc: &LoroDoc) -> Result<(), FileError> {
        let Some(stamp) = stamp_of(doc) else {
            return Ok(());
        };
        let stamp = stamp.to_string();
        if self.meta("schema").await.as_deref() == Some(stamp.as_str()) {
            return Ok(());
        }
        self.set_meta("schema", &stamp).await
    }
}

fn meta(key: &str, value: &str) -> Record {
    Record::Meta {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

/// A whole file of `records`.
fn image(records: &[Record]) -> Result<Vec<u8>, FileError> {
    let mut bytes = format::header();
    for r in records {
        bytes.extend(format::frame(r)?);
    }
    Ok(bytes)
}

/// The index `storage`'s records add up to, and the torn tail cut off.
async fn read(storage: &impl Storage) -> Result<(Index, Option<Recovery>), FileError> {
    let bytes = storage.read().await?;
    let parsed = format::parse(&bytes)?;
    let torn = parsed.torn(bytes.len());
    let recovered = if torn > 0 {
        let good = u64::try_from(parsed.good_len).unwrap_or(u64::MAX);
        storage.truncate(good).await?;
        Some(Recovery {
            dropped_bytes: u64::try_from(torn).unwrap_or(u64::MAX),
        })
    } else {
        None
    };
    Ok((Index::from_records(parsed.records), recovered))
}

/// Refuses a file a newer build wrote.
fn check_schema(index: &Index, kind: &DocKind) -> Result<(), FileError> {
    let found: u32 = index
        .meta("schema")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    if found > kind.stamp {
        return Err(FileError::Schema(format!(
            "this {} was written by a newer version (schema {found}, this build reads {})",
            kind.name, kind.stamp
        )));
    }
    Ok(())
}
