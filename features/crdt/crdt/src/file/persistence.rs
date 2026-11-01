//! A branch of a document file as a [`Persistence`], so a
//! [`CrdtDoc`](crate::CrdtDoc) — and the sync, registry and hosts built
//! on one — keeps its replica in the document's own file.

use async_trait::async_trait;
use loro::Frontiers;
use uuid::Uuid;

use super::{DocFile, FileError};
use crate::persistence::{PersistError, Persistence};

/// One branch of a [`DocFile`] behind the [`Persistence`] seam
/// ([`DocFile::persistence`]). The doc id the trait passes is ignored:
/// the file is one document.
///
/// - `load_snapshot` is the branch as the file holds it, whole;
///   `load_updates` is then empty.
/// - `append_update` is a step named "Sync", not listed as a version (a
///   `CrdtDoc` does not name its commits; edit through
///   [`Branch`](super::Branch) to name them).
/// - `compact` folds the branch from what the file holds (the snapshot
///   the doc offers is not needed: every update it wrote is there).
#[derive(Debug, Clone)]
pub struct BranchPersistence {
    file: DocFile,
    branch: String,
}

impl BranchPersistence {
    pub(super) const fn new(file: DocFile, branch: String) -> Self {
        Self { file, branch }
    }
}

// By value: an error-mapping adapter for `map_err`.
#[allow(clippy::needless_pass_by_value)]
fn persist(e: FileError) -> PersistError {
    PersistError::Backend(e.to_string())
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Persistence for BranchPersistence {
    async fn load_snapshot(&self, _doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        let doc = self.file.materialize(&self.branch).await.map_err(persist)?;
        if doc.oplog_vv().is_empty() {
            return Ok(None);
        }
        doc.export(loro::ExportMode::Snapshot)
            .map(Some)
            .map_err(|e| PersistError::Backend(format!("export snapshot: {e}")))
    }

    async fn write_snapshot(&self, _doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        // Whatever the snapshot holds that the file does not, as a step.
        self.append_update(Uuid::nil(), bytes).await
    }

    async fn append_update(&self, _doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        // No frontiers: working them out means loading the branch for
        // every update. A step without them is kept, and replayed, but
        // is not a version to open.
        self.file
            .append_step(&self.branch, "Sync", &Frontiers::default(), bytes.to_vec())
            .await
            .map_err(persist)
    }

    async fn load_updates(&self, _doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        Ok(Vec::new())
    }

    async fn compact(&self, _doc_id: Uuid, _snapshot: &[u8]) -> Result<(), PersistError> {
        self.file.fold(&self.branch).await.map_err(persist)
    }
}
