//! `FilePersistence` — file-backed [`Persistence`] for desktop clients
//! and small servers. One directory per doc:
//!
//! ```text
//! <root>/<doc_id>/
//!   snapshot.loro          the last compacted snapshot (if any)
//!   updates/<NNNNNNNNNN>.loro   the update log since, in commit order
//! ```
//!
//! Plain `std::fs` behind the async trait — update blobs are small and
//! writes are fire-and-forget from the doc's persistence task, so
//! blocking the executor for a microsecond write is the right trade
//! against pulling a whole async-fs stack.

use std::path::PathBuf;

use async_trait::async_trait;
use uuid::Uuid;

use crate::persistence::{PersistError, Persistence};

pub struct FilePersistence {
    root: PathBuf,
}

impl FilePersistence {
    /// Persist docs under `root` (created on demand).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn doc_dir(&self, doc_id: Uuid) -> PathBuf {
        self.root.join(doc_id.to_string())
    }

    fn snapshot_path(&self, doc_id: Uuid) -> PathBuf {
        self.doc_dir(doc_id).join("snapshot.loro")
    }

    fn updates_dir(&self, doc_id: Uuid) -> PathBuf {
        self.doc_dir(doc_id).join("updates")
    }

    /// Update file names in the log, sorted (zero-padded sequence
    /// numbers sort lexically = commit order).
    fn update_files(&self, doc_id: Uuid) -> Result<Vec<PathBuf>, PersistError> {
        let dir = self.updates_dir(doc_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(io_err)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "loro"))
            .collect();
        files.sort();
        Ok(files)
    }

    fn next_update_path(&self, doc_id: Uuid) -> Result<PathBuf, PersistError> {
        let dir = self.updates_dir(doc_id);
        std::fs::create_dir_all(&dir).map_err(io_err)?;
        let next = self
            .update_files(doc_id)?
            .last()
            .and_then(|p| p.file_stem()?.to_str()?.parse::<u64>().ok())
            .map_or(0, |n| n + 1);
        Ok(dir.join(format!("{next:010}.loro")))
    }
}

fn io_err(e: std::io::Error) -> PersistError {
    PersistError::Backend(format!("fs: {e}"))
}

#[async_trait]
impl Persistence for FilePersistence {
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        let path = self.snapshot_path(doc_id);
        if !path.exists() {
            return Ok(None);
        }
        std::fs::read(&path).map(Some).map_err(io_err)
    }

    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        let path = self.snapshot_path(doc_id);
        std::fs::create_dir_all(path.parent().expect("snapshot has a parent")).map_err(io_err)?;
        // Write-then-rename so a crash mid-write never truncates the
        // only copy of the doc.
        let tmp = path.with_extension("loro.tmp");
        std::fs::write(&tmp, bytes).map_err(io_err)?;
        std::fs::rename(&tmp, &path).map_err(io_err)
    }

    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        std::fs::write(self.next_update_path(doc_id)?, bytes).map_err(io_err)
    }

    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        self.update_files(doc_id)?
            .iter()
            .map(|p| std::fs::read(p).map_err(io_err))
            .collect()
    }

    async fn compact(&self, doc_id: Uuid, snapshot: &[u8]) -> Result<(), PersistError> {
        // Snapshot first — until the rename lands, the old snapshot +
        // full log are still intact; pruning only happens after.
        self.write_snapshot(doc_id, snapshot).await?;
        for path in self.update_files(doc_id)? {
            std::fs::remove_file(&path).map_err(io_err)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_update_compact_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = FilePersistence::new(dir.path());
        let id = Uuid::new_v4();

        assert!(p.load_snapshot(id).await.unwrap().is_none());
        assert!(p.load_updates(id).await.unwrap().is_empty());

        p.append_update(id, b"u1").await.unwrap();
        p.append_update(id, b"u2").await.unwrap();
        assert_eq!(
            p.load_updates(id).await.unwrap(),
            vec![b"u1".to_vec(), b"u2".to_vec()]
        );

        p.compact(id, b"snap").await.unwrap();
        assert_eq!(p.load_snapshot(id).await.unwrap().unwrap(), b"snap");
        assert!(p.load_updates(id).await.unwrap().is_empty());

        p.append_update(id, b"u3").await.unwrap();
        assert_eq!(p.load_updates(id).await.unwrap(), vec![b"u3".to_vec()]);
    }

    #[tokio::test]
    async fn doc_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();

        {
            let doc = crate::CrdtDoc::open(id, FilePersistence::new(dir.path()))
                .await
                .unwrap();
            doc.loro().get_map("root").insert("k", "v").unwrap();
            doc.loro().commit();
            // The persistence write is fire-and-forget through a channel;
            // yield until it lands.
            for _ in 0..100 {
                tokio::task::yield_now().await;
                if !FilePersistence::new(dir.path())
                    .load_updates(id)
                    .await
                    .unwrap()
                    .is_empty()
                {
                    break;
                }
            }
        }

        let doc = crate::CrdtDoc::open(id, FilePersistence::new(dir.path()))
            .await
            .unwrap();
        let v = doc
            .loro()
            .get_map("root")
            .get("k")
            .and_then(|v| v.into_value().ok())
            .and_then(|v| v.into_string().ok())
            .map(|s| s.to_string());
        assert_eq!(v.as_deref(), Some("v"));
    }
}
