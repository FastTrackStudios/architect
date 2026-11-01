//! One branch of a document, open: its Loro document and its named undo,
//! every finished step written to the file before it returns.

use loro::{ExportMode, LoroDoc, VersionVector};

use super::index::loro;
use super::{DocFile, FileError, MAIN, UNDO_STEPS, Version};
use crate::History;
use crate::kinds::DocKind;

/// A branch of a [`DocFile`], open to read and edit.
///
/// Edits go through [`Self::transact`]: one named step, on disk when it
/// returns. A change made straight on [`Self::loro`] is written by the
/// next step, or by [`Self::flush`].
pub struct Branch {
    file: DocFile,
    name: String,
    doc: LoroDoc,
    history: History,
    /// What of `doc` the file holds: the next step writes what is past it.
    saved: VersionVector,
}

impl std::fmt::Debug for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Branch")
            .field("name", &self.name)
            .field("history", &self.history)
            .finish_non_exhaustive()
    }
}

impl Branch {
    pub(super) fn new(file: DocFile, name: String, doc: LoroDoc) -> Self {
        let history = History::new(&doc, UNDO_STEPS);
        let saved = doc.oplog_vv();
        Self {
            file,
            name,
            doc,
            history,
            saved,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The branch's document, to read (and to sync: export from it, and
    /// bring others' updates in with [`Self::import`]).
    #[must_use]
    pub const fn loro(&self) -> &LoroDoc {
        &self.doc
    }

    /// The branch's named undo: what Undo and Redo would do, the steps.
    #[must_use]
    pub const fn history(&self) -> &History {
        &self.history
    }

    #[must_use]
    pub const fn file(&self) -> &DocFile {
        &self.file
    }

    /// Runs `change` as one step named `name`, and returns once the step
    /// is on disk. A change that changed nothing is no step.
    ///
    /// # Errors
    ///
    /// The file refused the write. The change stays applied, and the next
    /// step (or [`Self::flush`]) writes it.
    pub async fn transact<R>(
        &mut self,
        name: &str,
        change: impl FnOnce(&LoroDoc) -> R,
    ) -> Result<R, FileError> {
        let out = self.history.transact(&self.doc, name, change);
        self.flush(name).await?;
        Ok(out)
    }

    /// Undoes the last step, written as a step of its own; its name.
    pub async fn undo(&mut self) -> Result<Option<String>, FileError> {
        let Some(name) = self.history.undo() else {
            return Ok(None);
        };
        self.flush(&format!("Undo {name}")).await?;
        Ok(Some(name))
    }

    /// Redoes the last step undone, written as a step of its own; its name.
    pub async fn redo(&mut self) -> Result<Option<String>, FileError> {
        let Some(name) = self.history.redo() else {
            return Ok(None);
        };
        self.flush(&format!("Redo {name}")).await?;
        Ok(Some(name))
    }

    /// Writes whatever the file does not hold yet as one step named
    /// `label`; nothing if there is nothing.
    pub async fn flush(&mut self, label: &str) -> Result<(), FileError> {
        self.doc.commit();
        let now = self.doc.oplog_vv();
        if now == self.saved {
            return Ok(());
        }
        let update = self
            .doc
            .export(ExportMode::updates(&self.saved))
            .map_err(loro)?;
        self.file
            .append_step(&self.name, label, &self.doc.oplog_frontiers(), update)
            .await?;
        self.saved = now;
        Ok(())
    }

    /// Brings in `bytes` (a collaborator's update, another device's) and
    /// writes them as a step named `label`. Not undoable here: undo is
    /// this peer's own.
    pub async fn import(&mut self, bytes: &[u8], label: &str) -> Result<(), FileError> {
        self.doc.import(bytes).map_err(loro)?;
        self.flush(label).await
    }

    /// Names this point of the branch `name` (a version to come back to),
    /// without folding.
    pub async fn checkpoint(&mut self, name: &str) -> Result<Version, FileError> {
        self.flush("Edit").await?;
        self.file
            .checkpoint(&self.name, name, &self.doc.oplog_frontiers())
            .await
    }

    /// "Save": names this point `name` and folds the branch's steps into
    /// one snapshot. Nothing was unsaved before it — every step was
    /// already on disk — so Save is a name, and a smaller file.
    pub async fn save(&mut self, name: &str) -> Result<Version, FileError> {
        let version = self.checkpoint(name).await?;
        self.file.fold(&self.name).await?;
        Ok(version)
    }

    /// Forks a new branch `name` from here, and opens it.
    pub async fn fork(&mut self, name: &str) -> Result<Self, FileError> {
        self.flush("Edit").await?;
        self.file
            .new_branch(name, &self.name, &self.doc.oplog_frontiers())
            .await?;
        self.file.open_branch(name).await
    }

    /// Merges `other` (another branch, open beside this one) into this
    /// one, written as a step "Merge <other>". Loro merges: both sides'
    /// edits are kept.
    pub async fn merge(&mut self, other: &Self) -> Result<(), FileError> {
        let theirs = other
            .doc
            .export(ExportMode::updates(&self.doc.oplog_vv()))
            .map_err(loro)?;
        self.import(&theirs, &format!("Merge {}", other.name)).await
    }

    /// Merges the branch `name` as the file holds it into this one.
    pub async fn merge_branch(&mut self, name: &str) -> Result<(), FileError> {
        let theirs = self.file.materialize(name).await?;
        let bytes = theirs
            .export(ExportMode::updates(&self.doc.oplog_vv()))
            .map_err(loro)?;
        self.import(&bytes, &format!("Merge {name}")).await
    }

    /// Brings in the steps another handle on this branch wrote.
    pub async fn refresh(&mut self) -> Result<(), FileError> {
        let held = self.file.materialize(&self.name).await?;
        let bytes = held
            .export(ExportMode::updates(&self.doc.oplog_vv()))
            .map_err(loro)?;
        self.doc.import(&bytes).map_err(loro)?;
        // What came from the file is in the file; anything of this
        // handle's own not yet written still is not.
        self.saved.merge(&held.oplog_vv());
        Ok(())
    }

    /// Marks a new document as `kind` (its first step).
    pub(super) async fn stamp(&mut self, kind: &DocKind) -> Result<(), FileError> {
        kind.stamp(&self.doc);
        self.flush(&format!("New {}", kind.name)).await?;
        self.file.set_schema(&self.doc).await?;
        self.history.clear();
        Ok(())
    }

    /// Brings the branch up to `kind`'s schema, each migration written.
    pub(super) async fn upgrade(&mut self, kind: &DocKind) -> Result<(), FileError> {
        let before = crate::kinds::stamp_of(&self.doc);
        let at = kind.upgrade(&self.doc).map_err(FileError::Schema)?;
        if before != Some(at) {
            self.flush(&format!("Upgrade to schema {at}")).await?;
            if self.name == MAIN {
                self.file.set_schema(&self.doc).await?;
            }
        }
        self.history.clear();
        Ok(())
    }
}
