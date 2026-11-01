//! [`Placement`]: where each of an org's documents is kept on the device.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use super::DocMeta;
use super::driver::DocStorage;
use crate::file::{DiskStorage, DocFile, FileError, MAIN, Storage};
use crate::kinds::{DocKind, DocKinds};
use crate::{PersistError, Persistence};

/// Where an org's documents are kept: each in a document file
/// ([`DocFile`]).
///
/// A file is placed where the app keeps it (a rig's book beside its
/// library, a project's own file); anything else — made on another
/// device, or never placed — is under one directory, `<id>.<extension>`.
///
/// A catalogue row's `kind` names a registered [`DocKind`] by its id
/// (`studio.song`), which says what the file is and its extension. The
/// doc syncs through its file's `main` ([`DocFile::persistence`]), so
/// every step that arrives is durable in the file the person opens.
///
/// A file is made when a doc first needs one: stamped with its kind's
/// schema for a doc made on this device, empty for one made elsewhere
/// (its history, stamp included, comes from the device that made it —
/// an edit of the new file's own first would be another lineage). A
/// file already at the placed path that is not yet this doc (a book used
/// signed out) becomes it: its `main` is the doc's history from then on.
#[derive(Clone)]
pub struct Placement {
    kinds: Arc<DocKinds>,
    elsewhere: PathBuf,
    placed: Arc<Mutex<HashMap<Uuid, PathBuf>>>,
    /// Each open doc's file, so the app can name the steps it commits
    /// and open its other branches ([`Self::file`]).
    open: Arc<Mutex<HashMap<Uuid, DocFile>>>,
}

// By value: an error-mapping adapter for `map_err`.
#[allow(clippy::needless_pass_by_value)]
fn persist(e: FileError) -> PersistError {
    PersistError::Backend(e.to_string())
}

/// Whether `meta` is a doc made on this device and not registered yet:
/// the host has not stamped it (no creator, no revision).
const fn made_here(meta: &DocMeta) -> bool {
    meta.rev == 0 && meta.created_by.is_empty()
}

impl Placement {
    /// Documents of `kinds`, those not placed kept under `elsewhere`.
    #[must_use]
    pub fn new(elsewhere: impl Into<PathBuf>, kinds: DocKinds) -> Self {
        Self {
            kinds: Arc::new(kinds),
            elsewhere: elsewhere.into(),
            placed: Arc::new(Mutex::new(HashMap::new())),
            open: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// `doc_id`'s file is `file`. Say so before the driver opens the doc
    /// (before [`DocsDriver::create`](super::DocsDriver::create) or
    /// [`DocsDriver::open`](super::DocsDriver::open)).
    pub fn place(&self, doc_id: Uuid, file: impl Into<PathBuf>) {
        architect::lock(&self.placed).insert(doc_id, file.into());
    }

    /// Where `meta`'s doc is kept on this device.
    #[must_use]
    pub fn file_for(&self, meta: &DocMeta) -> PathBuf {
        if let Some(placed) = architect::lock(&self.placed).get(&meta.doc_id) {
            return placed.clone();
        }
        let ext = self
            .kinds
            .get(&meta.kind)
            .map(|k| k.extension.as_str())
            .filter(|e| !e.is_empty())
            .unwrap_or("doc");
        self.elsewhere.join(format!("{}.{ext}", meta.doc_id))
    }

    /// `doc_id`'s file, while the driver has the doc open.
    #[must_use]
    pub fn file(&self, doc_id: Uuid) -> Option<DocFile> {
        architect::lock(&self.open).get(&doc_id).cloned()
    }

    fn kind(&self, meta: &DocMeta) -> Result<&DocKind, PersistError> {
        self.kinds
            .get(&meta.kind)
            .ok_or_else(|| persist(FileError::UnknownKind(meta.kind.clone())))
    }

    /// The file at `meta`'s place, made if it is not there.
    async fn open_file(&self, meta: &DocMeta) -> Result<DocFile, PersistError> {
        let kind = self.kind(meta)?;
        let path = self.file_for(meta);
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| PersistError::Backend(format!("fs: {}: {e}", dir.display())))?;
        }
        let storage = DiskStorage::open(&path).map_err(persist)?;
        if storage.read().await.map_err(persist)?.is_empty() {
            let made = if made_here(meta) {
                DocFile::create_as(storage, kind, meta.doc_id).await
            } else {
                DocFile::adopt(storage, kind, meta.doc_id).await
            };
            return made.map_err(persist);
        }
        let file = DocFile::open(storage, kind).await.map_err(persist)?;
        if file.id().await != Some(meta.doc_id) {
            file.set_meta("id", &meta.doc_id.to_string())
                .await
                .map_err(persist)?;
        }
        Ok(file)
    }
}

#[async_trait::async_trait]
impl DocStorage for Placement {
    async fn persistence(&self, meta: &DocMeta) -> Result<Arc<dyn Persistence>, PersistError> {
        let file = self.open_file(meta).await?;
        let persistence = file.persistence(MAIN);
        architect::lock(&self.open).insert(meta.doc_id, file);
        Ok(Arc::new(persistence))
    }

    /// The file is made again, empty, for the hosts' history to arrive
    /// in: a fold keeps what the file holds, so emptying `main` through
    /// the [`Persistence`] would keep the old history.
    async fn discard(&self, meta: &DocMeta) -> Result<(), PersistError> {
        architect::lock(&self.open).remove(&meta.doc_id);
        let kind = self.kind(meta)?;
        let storage = DiskStorage::open(self.file_for(meta)).map_err(persist)?;
        DocFile::adopt(storage, kind, meta.doc_id)
            .await
            .map(drop)
            .map_err(persist)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::Lineage;

    fn meta(id: Uuid, kind: &str) -> DocMeta {
        DocMeta {
            doc_id: id,
            kind: kind.into(),
            name: String::new(),
            parent: None,
            created_by: String::new(),
            origin: None,
            created_us: 0,
            updated_us: 0,
            rev: 0,
            deleted: false,
            lineage: Lineage::Strict,
        }
    }

    #[test]
    fn a_doc_not_placed_is_kept_under_elsewhere_by_its_id_and_kind() {
        let mut kinds = DocKinds::new();
        kinds.register(DocKind::new("studio.song", "Song").extension("session"));
        let placement = Placement::new("/sync/docs", kinds);
        let (placed, loose, odd) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        placement.place(placed, "/songs/Unbreakable.session");
        assert_eq!(
            placement.file_for(&meta(placed, "studio.song")),
            PathBuf::from("/songs/Unbreakable.session")
        );
        assert_eq!(
            placement.file_for(&meta(loose, "studio.song")),
            PathBuf::from(format!("/sync/docs/{loose}.session"))
        );
        assert_eq!(
            placement.file_for(&meta(odd, "nobody.knows")),
            PathBuf::from(format!("/sync/docs/{odd}.doc")),
            "a kind this build does not register still has a place"
        );
    }
}
