//! Blobs by hash over vox: the [`BlobTransfer`] service a node serves
//! from its store ([`BlobTransferHost`]) and the [`Remote`] a node fetches
//! through ([`VoxRemote`]). The same service runs over every vox link —
//! a WebSocket to the org's server, an iroh connection to a peer
//! (`architect::iroh_link`), the in-process link in tests.

use std::io;
use std::sync::Arc;

use futures::future::BoxFuture;
use tokio::io::AsyncReadExt as _;

use super::stream_reader::StreamReader;
use super::{BlobError, BlobStore, Reader, Remote, Sha256};

/// Bytes per frame on the wire: well under every link's frame limit.
pub const CHUNK: usize = 256 * 1024;

/// Why a node would not send a blob.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet, thiserror::Error)]
#[repr(u8)]
pub enum TransferError {
    /// Not 64 hex digits.
    #[error("not a sha256: {0}")]
    BadHash(String),
    /// This node will not send this blob to this caller.
    #[error("refused")]
    Refused,
    /// The store failed.
    #[error("blob store: {0}")]
    Internal(String),
}

/// Read blobs by their sha256 (64 hex digits).
#[vox::service]
pub trait BlobTransfer {
    /// The blob's length, or `None` when this node does not hold it.
    async fn size(&self, hash: String) -> Result<Option<u64>, TransferError>;

    /// The blob's bytes from `offset` to its end, in order, on `down`.
    /// Returns after the last frame is queued; the stream closing is the
    /// end. A blob this node does not hold sends nothing.
    async fn read(
        &self,
        hash: String,
        offset: u64,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), TransferError>;
}

type Allow = dyn Fn(Sha256) -> bool + Send + Sync;

/// Serves [`BlobTransfer`] from a [`BlobStore`].
///
/// Mount it behind the lane's permissions gate like any service; [`Self::allow`] narrows
/// which blobs it sends (the gate's caller is in scope there, through
/// `architect::permissions_gate::caller`).
#[derive(Clone)]
pub struct BlobTransferHost {
    store: Arc<dyn BlobStore>,
    allow: Arc<Allow>,
}

impl std::fmt::Debug for BlobTransferHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobTransferHost").finish_non_exhaustive()
    }
}

impl BlobTransferHost {
    /// Serve every blob in `store`.
    pub fn new(store: Arc<dyn BlobStore>) -> Self {
        Self {
            store,
            allow: Arc::new(|_| true),
        }
    }

    /// Serve only the blobs `allow` accepts (say, those a document the
    /// caller may open points at). Refused blobs answer
    /// [`TransferError::Refused`].
    #[must_use]
    pub fn allow(mut self, allow: impl Fn(Sha256) -> bool + Send + Sync + 'static) -> Self {
        self.allow = Arc::new(allow);
        self
    }

    fn admit(&self, hex: &str) -> Result<Sha256, TransferError> {
        let hash = Sha256::from_hex(hex).map_err(|_| TransferError::BadHash(hex.to_owned()))?;
        if (self.allow)(hash) {
            Ok(hash)
        } else {
            Err(TransferError::Refused)
        }
    }
}

fn internal(e: &BlobError) -> TransferError {
    TransferError::Internal(e.to_string())
}

impl BlobTransfer for BlobTransferHost {
    async fn size(&self, hash: String) -> Result<Option<u64>, TransferError> {
        let hash = self.admit(&hash)?;
        match self.store.get(hash).await {
            Ok(blob) => Ok(Some(blob.len)),
            Err(BlobError::NotFound(_)) => Ok(None),
            Err(e) => Err(internal(&e)),
        }
    }

    async fn read(
        &self,
        hash: String,
        offset: u64,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), TransferError> {
        let hash = self.admit(&hash)?;
        let mut blob = match self.store.get_from(hash, offset).await {
            Ok(blob) => blob,
            Err(BlobError::NotFound(_)) => return Ok(()),
            Err(e) => return Err(internal(&e)),
        };
        loop {
            let mut buf = vec![0u8; CHUNK];
            let n = blob
                .reader
                .read(&mut buf)
                .await
                .map_err(|e| TransferError::Internal(e.to_string()))?;
            if n == 0 {
                return Ok(());
            }
            buf.truncate(n);
            if down.send(buf).await.is_err() {
                // The reader went away (seeked elsewhere, the link
                // dropped): not an error here.
                return Ok(());
            }
        }
    }
}

/// A peer's [`BlobTransfer`] as a [`Remote`].
#[derive(Clone)]
pub struct VoxRemote {
    client: BlobTransferClient,
}

impl std::fmt::Debug for VoxRemote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoxRemote").finish_non_exhaustive()
    }
}

impl VoxRemote {
    #[must_use]
    pub const fn new(client: BlobTransferClient) -> Self {
        Self { client }
    }
}

fn call_err(e: &vox::VoxError<TransferError>) -> io::Error {
    match e {
        vox::VoxError::User(e) if matches!(**e, TransferError::Refused) => {
            io::Error::new(io::ErrorKind::PermissionDenied, e.to_string())
        }
        vox::VoxError::User(e) => io::Error::other(e.to_string()),
        other => io::Error::new(io::ErrorKind::ConnectionAborted, format!("{other:?}")),
    }
}

impl Remote for VoxRemote {
    fn size(&self, hash: Sha256) -> BoxFuture<'_, io::Result<Option<u64>>> {
        Box::pin(async move {
            self.client
                .size(hash.to_hex())
                .await
                .map_err(|e| call_err(&e))
        })
    }

    fn read_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        Box::pin(async move {
            let (tx, rx) = vox::channel::<Vec<u8>>();
            let client = self.client.clone();
            // The call must run for the request (which binds `tx`) to go
            // out, and stays in flight while the frames arrive.
            let call = tokio::spawn(async move { client.read(hash.to_hex(), offset, tx).await });
            let frames = futures::stream::unfold((rx, Some(call)), |(mut rx, call)| async move {
                let call = call?;
                match rx.recv().await {
                    Ok(Some(frame)) => {
                        let mut owned = None;
                        let _ = frame.map(|f| owned = Some(f));
                        Some((Ok(owned.unwrap_or_default()), (rx, Some(call))))
                    }
                    // The stream closed: the call says whether that
                    // was the end or a failure.
                    _ => match call.await {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some((Err(call_err(&e)), (rx, None))),
                        Err(e) => Some((Err(io::Error::other(e)), (rx, None))),
                    },
                }
            });
            let reader: Reader = Box::pin(StreamReader::new(Box::pin(frames)));
            Ok(reader)
        })
    }
}
