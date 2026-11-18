//! Documents of a kind: a vault file whose extension is a registered
//! kind's, kept in the file itself (module docs, "Documents of a kind"),
//! and what happens when that file is deleted, renamed or copied.
//!
//! - **Deleted.** A file the catalog listed and finds gone
//!   ([`super::VaultCollab::notice_deletes`]) is tombstoned the way a
//!   note is: the document's `meta.deleted` ([`vault_doc::set_deleted`]),
//!   carried by the document to every other holder. The file was the
//!   document's storage, so from then on the document is kept under
//!   `<crdt root>/deleted-documents/<id>/` ([`DocumentStore`] switches
//!   over), whose folder is also the mark that the id is deleted here: an
//!   open of it never makes its file again, whoever names it. A holder
//!   that receives the tombstone removes its own file the same way
//!   ([`keep`]). A document deleted while closed has no history left here
//!   to tombstone; it is set the moment a peer's history reaches it, so
//!   the tombstone is a change after that history and never a second
//!   lineage.
//! - **Revived.** A file of a tombstoned id put back (a restore from the
//!   trash) is the document again on its next open: what the tombstone
//!   kept merges into the file and `meta.deleted` is cleared.
//! - **Renamed or copied.** One id is one file per node: the first path
//!   listed keeps it while its file still carries the id. A copy (same
//!   id, another path) is not listed and is said once; a rename is the
//!   old path gone and the new one found, so the document follows the
//!   file and is not tombstoned. A rename is not sent to other nodes:
//!   each keeps the file where it has it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use crdt::file::{BranchPersistence, DiskStorage, DocFile, MAIN};
use crdt::registry::DocRegistry;
use crdt::{CrdtDoc, DocKind, FilePersistence, PersistError, Persistence};
use uuid::Uuid;
use vault_live::Backend;
use vault_proto::{COLLAB_META_CONTAINER, COLLAB_META_DELETED, IfMatch, VaultSync};

use super::{Shared, doc_lock, document_path};

/// Under the crdt root: one folder per deleted document of a kind, its
/// tombstone and whatever history reached it since.
const DELETED_DOCUMENTS: &str = "deleted-documents";

/// Where the document `doc_id` of a kind is: its vault file, and whether a
/// listing found that file there (only then is its absence a delete).
pub struct Route {
    vault_id: String,
    path: String,
    listed: bool,
}

impl Route {
    fn is(&self, vault_id: &str, path: &str) -> bool {
        self.vault_id == vault_id && self.path == path
    }
}

pub type Routes = HashMap<Uuid, Route>;

/// The vault file the document `doc_id` of a kind is, while its kind is
/// registered.
pub fn document_route(shared: &Shared, doc_id: Uuid) -> Option<(String, String)> {
    let route = shared
        .documents
        .lock()
        .expect("documents lock poisoned")
        .get(&doc_id)
        .map(|r| (r.vault_id.clone(), r.path.clone()))?;
    shared.kinds.for_path(&route.1).is_some().then_some(route)
}

/// Whether the file `path` of `vault_id` is there and is the document
/// `doc_id`.
fn holds(backend: &Backend, vault_id: &str, path: &str, doc_id: Uuid) -> bool {
    document_path(backend, vault_id, path)
        .and_then(|abs| DocFile::peek(&abs).ok())
        .is_some_and(|header| header.id == doc_id)
}

/// A listing found the document `doc_id` in the file `path`: record it,
/// unless another file already is that document here (a copy carries the
/// id it was copied from), which is refused, and said once per file.
pub fn list(shared: &Shared, doc_id: Uuid, vault_id: &str, path: &str) -> bool {
    let mut routes = shared.documents.lock().expect("documents lock poisoned");
    if let Some(kept) = routes.get(&doc_id)
        && !kept.is(vault_id, path)
        && holds(&shared.backend, &kept.vault_id, &kept.path, doc_id)
    {
        let first = shared.copies.lock().expect("copies lock poisoned").insert((
            doc_id,
            vault_id.to_owned(),
            path.to_owned(),
        ));
        if first {
            tracing::warn!(
                doc = %doc_id,
                vault_id,
                "vault-collab: a second file of one document (a copy) is not synced; the first keeps it"
            );
        }
        return false;
    }
    routes.insert(
        doc_id,
        Route {
            vault_id: vault_id.to_owned(),
            path: path.to_owned(),
            listed: true,
        },
    );
    true
}

/// Another node names the document `doc_id` as its file `path`: record
/// that, unless this node keeps it already — at that path, or in a file
/// of its own elsewhere (renamed or copied here), which stays its one
/// file rather than a second one being made.
pub fn register(shared: &Shared, doc_id: Uuid, vault_id: &str, path: &str) {
    let mut routes = shared.documents.lock().expect("documents lock poisoned");
    if let Some(kept) = routes.get(&doc_id)
        && (kept.is(vault_id, path) || holds(&shared.backend, &kept.vault_id, &kept.path, doc_id))
    {
        return;
    }
    routes.insert(
        doc_id,
        Route {
            vault_id: vault_id.to_owned(),
            path: path.to_owned(),
            listed: false,
        },
    );
}

/// The documents whose listed file is gone: each is tombstoned.
pub fn notice_deletes(shared: &Arc<Shared>, registry: &DocRegistry) {
    let gone: Vec<Uuid> = {
        let mut routes = shared.documents.lock().expect("documents lock poisoned");
        routes
            .iter_mut()
            .filter_map(|(doc_id, route)| {
                // A path that could not be a document file now (through a
                // symlink, made private) is hidden, never deleted.
                let abs = document_path(&shared.backend, &route.vault_id, &route.path)?;
                if !route.listed || abs.exists() {
                    return None;
                }
                route.listed = false;
                Some(*doc_id)
            })
            .collect()
    };
    for doc_id in gone {
        tombstone(shared, registry, doc_id);
    }
}

fn deleted_root(shared: &Shared) -> PathBuf {
    shared.crdt_root.join(DELETED_DOCUMENTS)
}

/// Whether the document `doc_id` of a kind is deleted on this node.
pub fn is_tombstoned(shared: &Shared, doc_id: Uuid) -> bool {
    deleted_root(shared).join(doc_id.to_string()).is_dir()
}

/// The file of `doc_id` was deleted outside the document: mark it deleted
/// here, and set the tombstone in the document if it is open (closed, its
/// history went with the file; [`keep`] sets it when a peer's arrives).
fn tombstone(shared: &Arc<Shared>, registry: &DocRegistry, doc_id: Uuid) {
    if let Err(e) = std::fs::create_dir_all(deleted_root(shared).join(doc_id.to_string())) {
        tracing::warn!(doc = %doc_id, "vault-collab: a deleted document was not marked: {e}");
        return;
    }
    let (shared, registry) = (shared.clone(), registry.clone());
    tokio::spawn(async move {
        if !registry.is_open(doc_id).await {
            return;
        }
        // Fetched before its lock is taken (the rule in the module docs).
        let Ok(doc) = registry.doc(doc_id).await else {
            return;
        };
        let lock = doc_lock(&shared, doc_id);
        let _g = lock.lock().await;
        let loro = doc.loro();
        if !loro.oplog_vv().is_empty() && !vault_doc::is_deleted(loro) {
            vault_doc::set_deleted(loro, true);
            vault_doc::commit_as(loro, "deleted: vault file");
        }
    });
}

/// Opens the document of a kind kept in the vault file `path`: the file
/// itself is its storage (its `main` branch), made empty for `doc_id`
/// when it is not there yet — unless the document is deleted here, when
/// it is opened from its tombstone and no file is made.
pub async fn open_document(
    shared: &Arc<Shared>,
    doc_id: Uuid,
    vault_id: &str,
    path: &str,
) -> Result<CrdtDoc, PersistError> {
    let backend = |e: crdt::file::FileError| PersistError::Backend(format!("{path}: {e}"));
    let kind = shared
        .kinds
        .for_path(path)
        .ok_or_else(|| PersistError::Backend(format!("{path}: no kind registered for it")))?;
    let abs = document_path(&shared.backend, vault_id, path)
        .ok_or_else(|| PersistError::Backend(format!("{path}: not a file of the vault")))?;
    let tombstoned = is_tombstoned(shared, doc_id);
    if tombstoned && !abs.exists() {
        let gone = Arc::new(AtomicBool::new(true));
        let store = DocumentStore::new(shared, doc_id, kind, None, gone.clone());
        let doc = CrdtDoc::open(doc_id, store).await?;
        keep_document(shared, doc_id, &doc, gone);
        return Ok(doc);
    }
    let file = if abs.exists() {
        let file = DocFile::open(DiskStorage::open(&abs).map_err(backend)?, &kind)
            .await
            .map_err(backend)?;
        if file.id().await != Some(doc_id) {
            return Err(PersistError::Backend(format!(
                "{path} is another document than {doc_id}"
            )));
        }
        file
    } else {
        if let Some(dir) = abs.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| PersistError::Backend(format!("{path}: {e}")))?;
        }
        DocFile::adopt(DiskStorage::open(&abs).map_err(backend)?, &kind, doc_id)
            .await
            .map_err(backend)?
    };
    let bound = Bound {
        vault_id: vault_id.to_owned(),
        path: path.to_owned(),
        persistence: file.persistence(MAIN),
    };
    let gone = Arc::new(AtomicBool::new(false));
    let store = DocumentStore::new(shared, doc_id, kind, Some(bound), gone.clone());
    let doc = CrdtDoc::open(doc_id, store).await?;
    if tombstoned {
        revive(shared, doc_id, &doc).await?;
    }
    keep_document(shared, doc_id, &doc, gone);
    Ok(doc)
}

/// A file of a deleted document is back: what its tombstone kept merges
/// into the file, the tombstone comes off, and the mark goes.
async fn revive(shared: &Shared, doc_id: Uuid, doc: &CrdtDoc) -> Result<(), PersistError> {
    let tomb = FilePersistence::new(deleted_root(shared));
    let mut kept: Vec<Vec<u8>> = tomb.load_snapshot(doc_id).await?.into_iter().collect();
    kept.extend(tomb.load_updates(doc_id).await?);
    for bytes in kept {
        doc.apply_remote_durable(doc_id, &bytes).await?;
    }
    if vault_doc::is_deleted(doc.loro()) {
        vault_doc::set_deleted(doc.loro(), false);
        vault_doc::commit_as(doc.loro(), "revived: vault file");
    }
    std::fs::remove_dir_all(deleted_root(shared).join(doc_id.to_string()))
        .map_err(|e| PersistError::Backend(format!("{doc_id}: {e}")))
}

/// Watch a document of a kind for its tombstone ([`keep`]) on every
/// change: a local commit or one a peer sent. The subscription owns only a
/// sender (the module docs' rule), and the task a weak handle.
fn keep_document(shared: &Arc<Shared>, doc_id: Uuid, doc: &CrdtDoc, gone: Arc<AtomicBool>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    // Once on open: a file whose history already says deleted goes too.
    let _ = tx.send(());
    let bridge = tx.clone();
    doc.loro()
        .subscribe_root(Arc::new(move |_| {
            let _ = bridge.send(());
        }))
        .detach();
    drop(tx);
    let weak = doc.downgrade();
    let shared = shared.clone();
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            while rx.try_recv().is_ok() {}
            let Some(doc) = weak.upgrade() else { break };
            keep(&shared, doc_id, &doc, &gone).await;
        }
    });
}

/// Whether the document carries a tombstone at all, set or cleared.
fn has_tombstone(doc: &CrdtDoc) -> bool {
    doc.loro()
        .get_map(COLLAB_META_CONTAINER)
        .get(COLLAB_META_DELETED)
        .is_some()
}

/// One look at a document of a kind:
///
/// - deleted here and never tombstoned (closed when its file went), now
///   that a history reached it: the tombstone is set;
/// - tombstoned (here, or by a peer) while its file is still its storage:
///   it is kept under the crdt root from now on, and its file removed.
async fn keep(shared: &Arc<Shared>, doc_id: Uuid, doc: &CrdtDoc, gone: &AtomicBool) {
    let lock = doc_lock(shared, doc_id);
    let _g = lock.lock().await;
    let loro = doc.loro();
    if gone.load(Ordering::Acquire) && !has_tombstone(doc) && !loro.oplog_vv().is_empty() {
        vault_doc::set_deleted(loro, true);
        vault_doc::commit_as(loro, "deleted: vault file");
    }
    if gone.load(Ordering::Acquire) || !vault_doc::is_deleted(loro) {
        return;
    }
    // Switched before the snapshot: a commit in between lands under the
    // crdt root as an update, and the snapshot holds everything before.
    gone.store(true, Ordering::Release);
    let snapshot = match loro.export(crdt::loro::ExportMode::Snapshot) {
        Ok(snapshot) => snapshot,
        Err(e) => {
            gone.store(false, Ordering::Release);
            tracing::warn!(doc = %doc_id, "vault-collab: a deleted document's file kept: {e}");
            return;
        }
    };
    let tomb = FilePersistence::new(deleted_root(shared));
    if let Err(e) = tomb.write_snapshot(doc_id, &snapshot).await {
        gone.store(false, Ordering::Release);
        tracing::warn!(doc = %doc_id, "vault-collab: a deleted document's file kept: {e}");
        return;
    }
    remove_file(shared, doc_id).await;
}

/// Remove the file of the deleted document `doc_id`, if it is still there
/// and still that document.
async fn remove_file(shared: &Shared, doc_id: Uuid) {
    let Some((vault_id, path)) = document_route(shared, doc_id) else {
        return;
    };
    if let Some(route) = shared
        .documents
        .lock()
        .expect("documents lock poisoned")
        .get_mut(&doc_id)
    {
        route.listed = false;
    }
    if !holds(&shared.backend, &vault_id, &path, doc_id) {
        return;
    }
    let backend = shared.backend.clone();
    let removed =
        tokio::task::spawn_blocking(move || backend.delete_file(&vault_id, &path, IfMatch::Force))
            .await;
    match removed {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::warn!(doc = %doc_id, "vault-collab: a deleted document's file kept: {e}");
        }
        Err(e) => tracing::warn!(doc = %doc_id, "vault-collab: delete task failed: {e}"),
    }
}

/// The file a [`DocumentStore`] writes to, and where it is.
struct Bound {
    vault_id: String,
    path: String,
    persistence: BranchPersistence,
}

/// Where a document of a kind persists: its file's `main` branch while it
/// is live, following the file when it is renamed, and its tombstone's
/// folder under the crdt root once it is deleted (`gone`), so nothing it
/// writes after its delete (a sync, a compaction) makes the file again.
struct DocumentStore {
    shared: Arc<Shared>,
    doc_id: Uuid,
    kind: DocKind,
    file: tokio::sync::Mutex<Option<Bound>>,
    tomb: FilePersistence,
    gone: Arc<AtomicBool>,
}

impl DocumentStore {
    fn new(
        shared: &Arc<Shared>,
        doc_id: Uuid,
        kind: DocKind,
        file: Option<Bound>,
        gone: Arc<AtomicBool>,
    ) -> Self {
        Self {
            shared: shared.clone(),
            doc_id,
            kind,
            file: tokio::sync::Mutex::new(file),
            tomb: FilePersistence::new(deleted_root(shared)),
            gone,
        }
    }

    /// The file's branch to write to now, `None` once the document is
    /// deleted. A route that moved (the file renamed) is followed when the
    /// file there is this document.
    async fn file(&self) -> Option<BranchPersistence> {
        if self.gone.load(Ordering::Acquire) {
            return None;
        }
        let mut slot = self.file.lock().await;
        let bound = slot.as_mut()?;
        if let Some((vault_id, path)) = document_route(&self.shared, self.doc_id)
            && !(vault_id == bound.vault_id && path == bound.path)
            && let Some(persistence) = self.bind(&vault_id, &path).await
        {
            *bound = Bound {
                vault_id,
                path,
                persistence,
            };
        }
        let persistence = bound.persistence.clone();
        drop(slot);
        Some(persistence)
    }

    async fn file_is_there(&self) -> bool {
        let slot = self.file.lock().await;
        slot.as_ref()
            .and_then(|b| document_path(&self.shared.backend, &b.vault_id, &b.path))
            .is_some_and(|abs| abs.exists())
    }

    async fn bind(&self, vault_id: &str, path: &str) -> Option<BranchPersistence> {
        let abs = document_path(&self.shared.backend, vault_id, path)?;
        if !abs.exists() {
            return None;
        }
        let file = DocFile::open(DiskStorage::open(&abs).ok()?, &self.kind)
            .await
            .ok()?;
        (file.id().await == Some(self.doc_id)).then(|| file.persistence(MAIN))
    }
}

#[async_trait]
impl Persistence for DocumentStore {
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        match self.file().await {
            Some(file) => file.load_snapshot(doc_id).await,
            None => self.tomb.load_snapshot(doc_id).await,
        }
    }

    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        match self.file().await {
            Some(file) => file.write_snapshot(doc_id, bytes).await,
            None => self.tomb.write_snapshot(doc_id, bytes).await,
        }
    }

    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        match self.file().await {
            Some(file) => file.append_update(doc_id, bytes).await,
            None => self.tomb.append_update(doc_id, bytes).await,
        }
    }

    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        match self.file().await {
            Some(file) => file.load_updates(doc_id).await,
            None => self.tomb.load_updates(doc_id).await,
        }
    }

    async fn compact(&self, doc_id: Uuid, snapshot: &[u8]) -> Result<(), PersistError> {
        match self.file().await {
            // Folding rewrites the file whole: over a file deleted outside
            // the document (not noticed yet) that would make it again.
            Some(_) if !self.file_is_there().await => Ok(()),
            Some(file) => file.compact(doc_id, snapshot).await,
            None => self.tomb.compact(doc_id, snapshot).await,
        }
    }
}
