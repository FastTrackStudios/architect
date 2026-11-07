//! The bytes a device can serve: its blob store, and its working files.
//!
//! A project's current audio stays where the DAW wrote it; there is no
//! second copy in the store. [`TreeStore`] answers a hash from the store
//! when it holds it, else from the working file the index says holds it
//! (checked against the index's stat first, so a file changed since is
//! never served as the old content).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use architect::blob::{Blob, BlobError, BlobStore, LocalFs, Reader, Remote, Sha256};
use futures_util::future::BoxFuture;

use super::scan::Stat;

/// Hash → the working file holding it.
#[derive(Clone, Default)]
pub struct InPlace {
    files: Arc<Mutex<HashMap<Sha256, (PathBuf, Stat)>>>,
}

impl InPlace {
    /// `path` holds `hash` while its stat is `stat`.
    pub fn insert(&self, hash: Sha256, path: PathBuf, stat: Stat) {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(hash, (path, stat));
    }

    /// The file holding `hash`, if it still does.
    #[must_use]
    pub fn find(&self, hash: &Sha256) -> Option<PathBuf> {
        let (path, stat) = self
            .files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(hash)
            .cloned()?;
        let now = std::fs::metadata(&path).ok().map(|m| Stat::of(&m))?;
        (now == stat).then_some(path)
    }
}

/// [`BlobStore`] over a [`LocalFs`] and the working files.
#[derive(Clone)]
pub struct TreeStore {
    pub local: LocalFs,
    pub in_place: InPlace,
}

impl TreeStore {
    #[must_use]
    pub fn new(local: LocalFs) -> Self {
        Self {
            local,
            in_place: InPlace::default(),
        }
    }

    /// The file holding `hash`: the store's, else a working file.
    #[must_use]
    pub fn file_for(&self, hash: &Sha256) -> Option<PathBuf> {
        let stored = self.local.path_for(hash);
        if stored.is_file() {
            return Some(stored);
        }
        self.in_place.find(hash)
    }
}

impl BlobStore for TreeStore {
    fn put(
        &self,
        expected: Option<Sha256>,
        body: Reader,
    ) -> BoxFuture<'_, Result<Sha256, BlobError>> {
        self.local.put(expected, body)
    }

    fn get(&self, hash: Sha256) -> BoxFuture<'_, Result<Blob, BlobError>> {
        self.get_from(hash, 0)
    }

    fn has(&self, hash: Sha256) -> BoxFuture<'_, Result<bool, BlobError>> {
        Box::pin(async move { Ok(self.file_for(&hash).is_some()) })
    }

    fn get_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, Result<Blob, BlobError>> {
        Box::pin(async move {
            let path = self.file_for(&hash).ok_or_else(|| {
                BlobError::Source(std::io::Error::from(std::io::ErrorKind::NotFound))
            })?;
            open_from(&path, offset).await
        })
    }
}

/// Another device's store as a fetch remote, in one process (tests, and
/// a device whose two apps share a disk).
impl Remote for TreeStore {
    fn size(&self, hash: Sha256) -> BoxFuture<'_, std::io::Result<Option<u64>>> {
        Box::pin(async move {
            Ok(match self.file_for(&hash) {
                Some(path) => Some(tokio::fs::metadata(path).await?.len()),
                None => None,
            })
        })
    }

    fn read_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, std::io::Result<Reader>> {
        Box::pin(async move {
            let path = self
                .file_for(&hash)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
            open_from(&path, offset)
                .await
                .map(|b| b.reader)
                .map_err(std::io::Error::other)
        })
    }
}

/// A file's bytes from `offset`.
///
/// # Errors
///
/// The file cannot be opened or seeked.
pub async fn open_from(path: &std::path::Path, offset: u64) -> Result<Blob, BlobError> {
    use tokio::io::AsyncSeekExt as _;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(BlobError::Source)?;
    let total = file.metadata().await.map_err(BlobError::Source)?.len();
    let offset = offset.min(total);
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(BlobError::Source)?;
    Ok(Blob {
        len: total.saturating_sub(offset),
        reader: Box::pin(file),
    })
}
