//! What a [`LocalFs`] keeps: pins (blobs kept offline whatever points at
//! them) and the collection of blobs nothing keeps.
//!
//! A pin is an empty file `<root>/.pins/<hex>`, so it survives restarts
//! and needs no index. [`LocalFs::collect`] removes every blob that is
//! neither in the live set the caller passes (the blobs its documents
//! point at, `crdt::blobs::live`) nor pinned, along with abandoned
//! partial fetches of such blobs.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{BlobError, LocalFs, Sha256, io_at};

/// What a [`LocalFs::collect`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Collected {
    /// Blobs removed.
    pub removed: Vec<Sha256>,
    /// Bytes freed (blobs and partial fetches).
    pub freed: u64,
    /// Blobs no document points at that a pin kept.
    pub pinned: usize,
}

impl LocalFs {
    fn pins_dir(&self) -> PathBuf {
        self.root.join(".pins")
    }

    /// Where a fetch by hash stages the blob before it moves into place
    /// (`<root>/.partial/<hex>`; its bytes arrive in `<hex>.part`).
    #[must_use]
    pub fn partial_path(&self, hash: &Sha256) -> PathBuf {
        self.root.join(".partial").join(hash.to_hex())
    }

    /// Keep `hash` on this device: [`Self::collect`] never removes it,
    /// whether or not anything points at it. Pinning a blob that is not
    /// here yet is allowed (fetch it to have it offline).
    pub async fn pin(&self, hash: Sha256) -> Result<(), BlobError> {
        let dir = self.pins_dir();
        tokio::fs::create_dir_all(&dir).await.map_err(io_at(&dir))?;
        let path = dir.join(hash.to_hex());
        tokio::fs::write(&path, b"").await.map_err(io_at(&path))
    }

    /// Stop keeping `hash` for its own sake; the next collection removes
    /// it if nothing points at it.
    pub async fn unpin(&self, hash: Sha256) -> Result<(), BlobError> {
        let path = self.pins_dir().join(hash.to_hex());
        match tokio::fs::remove_file(&path).await {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_at(&path)(e)),
            _ => Ok(()),
        }
    }

    /// Whether `hash` is pinned.
    pub async fn is_pinned(&self, hash: Sha256) -> Result<bool, BlobError> {
        let path = self.pins_dir().join(hash.to_hex());
        tokio::fs::try_exists(&path).await.map_err(io_at(&path))
    }

    /// Every pinned hash.
    pub async fn pinned(&self) -> Result<HashSet<Sha256>, BlobError> {
        Ok(hashes_in(&self.pins_dir())
            .await?
            .into_iter()
            .map(|(hash, _)| hash)
            .collect())
    }

    /// Every blob in the store.
    pub async fn list(&self) -> Result<Vec<Sha256>, BlobError> {
        Ok(self
            .stored()
            .await?
            .into_iter()
            .map(|(hash, _)| hash)
            .collect())
    }

    /// `(hash, path)` of every stored blob.
    async fn stored(&self) -> Result<Vec<(Sha256, PathBuf)>, BlobError> {
        let mut out = Vec::new();
        let mut dirs = match tokio::fs::read_dir(&self.root).await {
            Ok(dirs) => dirs,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(io_at(&self.root)(e)),
        };
        while let Some(dir) = dirs.next_entry().await.map_err(io_at(&self.root))? {
            let name = dir.file_name();
            let is_prefix = name.len() == 2 && !name.to_string_lossy().starts_with('.');
            if is_prefix && dir.file_type().await.is_ok_and(|t| t.is_dir()) {
                out.extend(hashes_in(&dir.path()).await?);
            }
        }
        Ok(out)
    }

    /// Remove every blob that is not in `live` and not pinned, and every
    /// partial fetch of one. Blobs and partials younger than `grace` are
    /// left alone, so one stored (or fetching) since `live` was computed
    /// is not lost.
    ///
    /// `live` must be every blob every document on this device points
    /// at (`crdt::blobs::live`); a blob left out of it is gone.
    pub async fn collect(
        &self,
        live: &HashSet<Sha256>,
        grace: Duration,
    ) -> Result<Collected, BlobError> {
        let pinned = self.pinned().await?;
        let mut report = Collected::default();
        for (hash, path) in self.stored().await? {
            if live.contains(&hash) {
                continue;
            }
            if pinned.contains(&hash) {
                report.pinned = report.pinned.saturating_add(1);
                continue;
            }
            if let Some(len) = remove_if_older(&path, grace).await? {
                report.removed.push(hash);
                report.freed = report.freed.saturating_add(len);
            }
        }
        let partial = self.root.join(".partial");
        for path in files_in(&partial).await? {
            let stem = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.split('.').next())
                .and_then(|hex| Sha256::from_hex(hex).ok());
            let kept = stem.is_some_and(|h| live.contains(&h) || pinned.contains(&h));
            if !kept && let Some(len) = remove_if_older(&path, grace).await? {
                report.freed = report.freed.saturating_add(len);
            }
        }
        report.removed.sort();
        Ok(report)
    }
}

/// The files directly in `dir` (none when it does not exist).
async fn files_in(dir: &Path) -> Result<Vec<PathBuf>, BlobError> {
    let mut out = Vec::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(io_at(dir)(e)),
    };
    while let Some(entry) = entries.next_entry().await.map_err(io_at(dir))? {
        if entry.file_type().await.is_ok_and(|t| t.is_file()) {
            out.push(entry.path());
        }
    }
    Ok(out)
}

/// The files in `dir` named by a hash, with their paths.
async fn hashes_in(dir: &Path) -> Result<Vec<(Sha256, PathBuf)>, BlobError> {
    Ok(files_in(dir)
        .await?
        .into_iter()
        .filter_map(|path| {
            let hash = Sha256::from_hex(path.file_name()?.to_str()?).ok()?;
            Some((hash, path))
        })
        .collect())
}

/// Remove `path` unless it changed within `grace`; its length if removed.
async fn remove_if_older(path: &Path, grace: Duration) -> Result<Option<u64>, BlobError> {
    let meta = match tokio::fs::metadata(path).await {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_at(path)(e)),
    };
    let age = meta
        .modified()
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .unwrap_or_default();
    if age < grace {
        return Ok(None);
    }
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(Some(meta.len())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_at(path)(e)),
    }
}
