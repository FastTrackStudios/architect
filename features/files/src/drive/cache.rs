//! The drive's cache: the one place a file's bytes are fetched for the
//! mount, read while they arrive, and evicted.
//!
//! Every path that changes what this device holds goes through here
//! ([`Cache::begin`], [`Cache::evict`]), so this device's attestations
//! (`Engine::attest`, `Engine::evict_blob`) follow the mount's fetches
//! and evictions the moment they happen.

use std::collections::HashMap;
use std::os::unix::fs::FileExt as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use architect::blob::Sha256;
use tokio::io::AsyncReadExt as _;
use tokio::runtime::Handle;

use crate::sync::Engine;

/// How a fetch for the mount went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch {
    Running,
    Done,
    /// No peer gave it: offline, or nobody holds it.
    Failed(String),
}

/// The bytes the mount reads, fetched on open and kept for offline use.
#[derive(Clone)]
pub struct Cache {
    engine: Engine,
    rt: Handle,
    fetches: Arc<Mutex<HashMap<Sha256, Fetch>>>,
}

impl Cache {
    #[must_use]
    pub fn new(engine: Engine, rt: Handle) -> Self {
        Self {
            engine,
            rt,
            fetches: Arc::default(),
        }
    }

    fn set(&self, hash: Sha256, state: Fetch) {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(hash, state);
    }

    /// The file on this device holding `hash` whole, if any.
    #[must_use]
    pub fn here(&self, hash: &Sha256) -> Option<PathBuf> {
        self.engine.store.file_for(hash)
    }

    /// Start fetching `hash` in the background unless it is here or on its
    /// way: an opened file lands whole and stays, even if only its head is
    /// read. The hook for a holder's attestation of a newly cached blob.
    pub fn begin(&self, hash: Sha256) {
        if self.here(&hash).is_some() {
            return;
        }
        {
            let mut fetches = self.fetches.lock().unwrap_or_else(PoisonError::into_inner);
            if fetches.get(&hash) == Some(&Fetch::Running) {
                return;
            }
            fetches.insert(hash, Fetch::Running);
        }
        let this = self.clone();
        self.rt.spawn(async move {
            let state = match this.engine.fetcher.fetch(hash).await {
                Ok(()) => {
                    // The fetcher checked every byte: attested `cached`,
                    // verified now, the moment it lands.
                    this.engine.note_verified(&hash.to_hex());
                    this.engine.attest(&[hash]).await;
                    Fetch::Done
                }
                Err(e) => Fetch::Failed(e.to_string()),
            };
            this.set(hash, state);
        });
    }

    /// How the fetch of `hash` stands.
    #[must_use]
    pub fn state(&self, hash: &Sha256) -> Option<Fetch> {
        if self.here(hash).is_some() {
            return Some(Fetch::Done);
        }
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(hash)
            .cloned()
    }

    /// `len` bytes of `hash` from `offset`: from disk when it is here,
    /// else streamed from a peer as they arrive (the first bytes come
    /// before the whole file). Blocking: call it off the runtime.
    ///
    /// # Errors
    ///
    /// The bytes cannot be had: no peer answered (offline), or a read failed.
    pub fn read(&self, hash: Sha256, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        if let Some(file) = self.here(&hash) {
            let file = std::fs::File::open(file)?;
            let mut buf = vec![0u8; len];
            let mut got = 0;
            while got < len {
                let n = file.read_at(
                    buf.get_mut(got..).unwrap_or_default(),
                    offset.saturating_add(u64::try_from(got).unwrap_or(u64::MAX)),
                )?;
                if n == 0 {
                    break;
                }
                got = got.saturating_add(n);
            }
            buf.truncate(got);
            return Ok(buf);
        }
        self.begin(hash);
        let fetcher = self.engine.fetcher.clone();
        self.rt.block_on(async move {
            let blob = fetcher.open_at(hash, offset).await.map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::NotConnected, e.to_string())
            })?;
            let mut buf = Vec::with_capacity(len);
            blob.reader
                .take(u64::try_from(len).unwrap_or(u64::MAX))
                .read_to_end(&mut buf)
                .await?;
            Ok(buf)
        })
    }

    /// The whole of `hash` into `into` (a write's starting copy).
    ///
    /// # Errors
    ///
    /// The bytes cannot be had, or `into` cannot be written.
    pub fn copy_to(&self, hash: Sha256, into: &std::path::Path) -> std::io::Result<()> {
        self.begin(hash);
        let fetcher = self.engine.fetcher.clone();
        self.rt
            .block_on(async move { fetcher.fetch(hash).await })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotConnected, e.to_string()))?;
        let from = self
            .here(&hash)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        let mut src = std::fs::File::open(from)?;
        let mut dst = std::fs::File::create(into)?;
        std::io::copy(&mut src, &mut dst)?;
        Ok(())
    }

    /// Drop `hash` from this device's cache (not a pinned root's working
    /// file), through [`Engine::evict_blob`]: refused when it would leave
    /// a file under its copy policy, attested `evicting` then withdrawn.
    /// Blocking: call it off the runtime.
    ///
    /// # Errors
    ///
    /// Under policy (`PermissionDenied`, naming the files), or the file
    /// cannot be removed.
    pub fn evict(&self, hash: &Sha256) -> std::io::Result<()> {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(hash);
        let engine = self.engine.clone();
        let hash = *hash;
        self.rt
            .block_on(async move { engine.evict_blob(hash).await })
            .map_err(|e| match e {
                crate::sync::EngineError::BelowPolicy(_) => {
                    std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.to_string())
                }
                other => std::io::Error::other(other.to_string()),
            })
    }

    /// The bytes the cache holds (the blob store, working copies aside).
    #[must_use]
    pub fn size(&self) -> u64 {
        dir_size(self.engine.store.local.root())
    }
}

fn dir_size(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => dir_size(&e.path()),
                Ok(_) => e.metadata().map_or(0, |m| m.len()),
                Err(_) => 0,
            })
            .fold(0_u64, u64::saturating_add)
    })
}
