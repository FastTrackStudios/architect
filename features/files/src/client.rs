//! [`FilesClient`]: the files feature as an app's store.
//!
//! | want | call |
//! |---|---|
//! | somewhere to keep things | [`FilesClient::ensure_root`] |
//! | save bytes, safely | [`FilesClient::put`] with a [`Save`] |
//! | read them back | [`FilesClient::get`], [`FilesClient::read_range`] |
//! | follow a pinned reference | [`FilesClient::resolve`], [`FilesClient::get_ref`] |
//! | hear what changed | [`FilesClient::events`] |
//! | play it in a browser | [`FilesClient::ticket`], then [`crate::web::content_url`] |
//! | its proxy, poster, peaks | [`FilesClient::renditions`], [`crate::web::rendition_url`] |
//!
//! It opens no connection: it is built from a vox `Caller` (a browser's
//! WebSocket, a native client's, a test's in-process link), and drives
//! its futures with `futures` combinators, never a runtime's `spawn`, so
//! the same code runs on wasm.
//!
//! A save names its bytes by their sha256 first, so bytes the org
//! already holds are not sent; what is sent is checked against it.

use std::future::Future;

use architect::vox;
use futures_util::future::{Either, join, select};
use sha2::Digest as _;

use crate::error::FilesFault;
use crate::id::{ContentId, RootId, UploadId, VersionId};
use crate::model::{
    ByteRange, Entry, Expect, FilesEvent, OnConflict, ReadRequest, Root, UploadFrame, UploadSpec,
};
use crate::path::RootPath;
use crate::rendition::{RenditionSet, Ticket};
use crate::service::{FilesServiceClient, FilesServiceStreamClient};

/// Bytes per upload frame: small enough that vox's credit paces a slow
/// server, big enough that framing is not the cost.
pub const FRAME: usize = 256 * 1024;

/// How many times [`FilesClient::put`] sends again what the server still
/// reports missing, before giving up.
const RESENDS: usize = 3;

/// What went wrong, split by what the caller does about it.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The server refused, for a reason in the domain: act on it.
    #[error("{0}")]
    Fault(FilesFault),
    /// The call did not complete (connection, a gate's refusal): retry
    /// or reconnect.
    #[error("transport: {0}")]
    Transport(String),
    /// A byte stream stopped short.
    #[error("stream: {0}")]
    Stream(String),
}

impl ClientError {
    #[must_use]
    pub const fn fault(&self) -> Option<&FilesFault> {
        match self {
            Self::Fault(f) => Some(f),
            _ => None,
        }
    }

    /// A safe save refused because the file moved on.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        matches!(self, Self::Fault(FilesFault::Stale { .. }))
    }
}

impl From<vox::VoxError<FilesFault>> for ClientError {
    fn from(e: vox::VoxError<FilesFault>) -> Self {
        match e {
            vox::VoxError::User(fault) => Self::Fault(*fault),
            other => Self::Transport(format!("{other:?}")),
        }
    }
}

impl From<FilesFault> for ClientError {
    fn from(fault: FilesFault) -> Self {
        Self::Fault(fault)
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// How a save treats what is already there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub on_conflict: OnConflict,
    pub expect: Option<Expect>,
}

impl Save {
    /// Only if nothing is there.
    #[must_use]
    pub const fn create_only() -> Self {
        Self {
            on_conflict: OnConflict::Fail,
            expect: Some(Expect::Absent),
        }
    }

    /// Replace exactly the content last seen as `etag`, and nothing
    /// newer: the save an editor makes.
    #[must_use]
    pub const fn replacing(etag: ContentId) -> Self {
        Self {
            on_conflict: OnConflict::Replace,
            expect: Some(Expect::Content(etag)),
        }
    }

    /// Replace whatever is there (a new version; the old one stays in
    /// history). Last writer wins.
    #[must_use]
    pub const fn overwrite() -> Self {
        Self {
            on_conflict: OnConflict::Replace,
            expect: None,
        }
    }

    /// Land beside an occupant under a derived name.
    #[must_use]
    pub const fn keep_both() -> Self {
        Self {
            on_conflict: OnConflict::KeepBoth,
            expect: None,
        }
    }
}

/// What a pinned reference resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The path holds the pinned bytes (or the reference pinned none).
    Current(Entry),
    /// The path holds other bytes now; the pinned ones are in its
    /// history ([`FilesClient::get_ref`]).
    Moved { now: Entry, pinned: ContentId },
    /// Nothing at the path.
    Missing { pinned: Option<ContentId> },
}

/// The sha256 of `bytes`, as the store names them.
#[must_use]
pub fn content_of(bytes: &[u8]) -> ContentId {
    use std::fmt::Write as _;
    let digest = sha2::Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for b in digest {
        let _ = write!(hex, "{b:02x}");
    }
    ContentId(hex)
}

/// One org's files, over one connection.
#[derive(Clone)]
pub struct FilesClient {
    pub service: FilesServiceClient,
    pub stream: FilesServiceStreamClient,
}

impl FilesClient {
    /// Both clients over `caller` (an org's lane, `/org/{slug}/vox`).
    #[must_use]
    pub fn new(caller: vox::Caller) -> Self {
        Self {
            service: FilesServiceClient::new(caller.clone()),
            stream: FilesServiceStreamClient::new(caller),
        }
    }

    // ── Roots ──────────────────────────────────────────────────────

    /// The root at `dir`, made if there is none.
    pub async fn ensure_root(&self, dir: &str, name: &str) -> Result<Root> {
        Ok(self
            .service
            .ensure_root(dir.to_owned(), name.to_owned())
            .await?)
    }

    pub async fn entry(&self, root: RootId, path: &str) -> Result<Entry> {
        Ok(self.service.entry(root, parse(path)?).await?)
    }

    pub async fn browse(&self, root: RootId, path: &str) -> Result<Vec<Entry>> {
        Ok(self.service.browse(root, parse(path)?).await?)
    }

    // ── Saving ─────────────────────────────────────────────────────

    /// Save `bytes` at `path`. Sends only what the server needs (nothing,
    /// when the org holds these bytes), resends what did not arrive, and
    /// lands it as `save` says.
    pub async fn put(&self, root: RootId, path: &str, bytes: &[u8], save: Save) -> Result<Entry> {
        let plan = self
            .service
            .begin_upload(UploadSpec {
                root_id: root,
                path: parse(path)?,
                size: len_u64(bytes.len()),
                content: Some(content_of(bytes)),
                expect: save.expect.clone(),
            })
            .await?;
        let mut needed = plan.needed;
        for _ in 0..=RESENDS {
            if needed.is_empty() {
                break;
            }
            needed = self.send_ranges(plan.upload_id, bytes, &needed).await?;
        }
        if !needed.is_empty() {
            let _ = self.service.abort_upload(plan.upload_id).await;
            return Err(ClientError::Stream(format!(
                "{path}: {} range(s) still missing after {RESENDS} resends",
                needed.len()
            )));
        }
        Ok(self
            .service
            .complete_upload(plan.upload_id, save.on_conflict)
            .await?)
    }

    /// Send `ranges` of `bytes` on one call; what the server still needs
    /// after it.
    async fn send_ranges(
        &self,
        upload: UploadId,
        bytes: &[u8],
        ranges: &[ByteRange],
    ) -> Result<Vec<ByteRange>> {
        let (tx, rx) = vox::channel::<UploadFrame>();
        let mut frames: Vec<(u64, Vec<u8>)> = Vec::new();
        for range in ranges {
            let start = usize::try_from(range.start).unwrap_or(usize::MAX);
            let end = usize::try_from(range.end).unwrap_or(usize::MAX);
            let Some(slice) = bytes.get(start..end.min(bytes.len())) else {
                continue;
            };
            let mut offset = range.start;
            for piece in slice.chunks(FRAME) {
                frames.push((offset, piece.to_vec()));
                offset = offset.saturating_add(len_u64(piece.len()));
            }
        }
        let sending = async move {
            for (offset, bytes) in frames {
                if tx.send(UploadFrame::Chunk { offset, bytes }).await.is_err() {
                    // The call side says why.
                    return;
                }
            }
            let _ = tx.send(UploadFrame::Finished).await;
        };
        let (received, ()) = join(self.service.send_bytes(upload, rx), sending).await;
        Ok(received?.needed)
    }

    // ── Reading ────────────────────────────────────────────────────

    /// A file's current bytes.
    pub async fn get(&self, root: RootId, path: &str) -> Result<Vec<u8>> {
        self.read(ReadRequest {
            root_id: root,
            path: parse(path)?,
            version: None,
            range: None,
        })
        .await
    }

    /// `[start, end)` of a file's current bytes (clamped to the file).
    pub async fn read_range(
        &self,
        root: RootId,
        path: &str,
        start: u64,
        end: u64,
    ) -> Result<Vec<u8>> {
        self.read(ReadRequest {
            root_id: root,
            path: parse(path)?,
            version: None,
            range: Some(ByteRange { start, end }),
        })
        .await
    }

    /// Any read, collected.
    pub async fn read(&self, request: ReadRequest) -> Result<Vec<u8>> {
        let (tx, mut rx) = vox::channel::<Vec<u8>>();
        let collecting = async move {
            let mut out = Vec::new();
            loop {
                match rx.recv().await {
                    Ok(Some(chunk)) => {
                        let _ = chunk.map(|c| out.extend_from_slice(&c));
                    }
                    Ok(None) => return Ok(out),
                    Err(e) => return Err(ClientError::Transport(format!("{e:?}"))),
                }
            }
        };
        let (done, out) = join(self.service.read(request, tx), collecting).await;
        let done = done?;
        let out = out?;
        if len_u64(out.len()) != done.range.len() {
            return Err(ClientError::Stream(format!(
                "the read promised {} bytes and sent {}",
                done.range.len(),
                out.len()
            )));
        }
        Ok(out)
    }

    // ── References ─────────────────────────────────────────────────

    /// What a reference (a path, and the content it was pinned to) points
    /// at now.
    pub async fn resolve(
        &self,
        root: RootId,
        path: &str,
        pinned: Option<ContentId>,
    ) -> Result<Resolved> {
        match self.service.entry(root, parse(path)?).await {
            Ok(now) => Ok(match pinned {
                Some(pin) if now.content.as_ref() != Some(&pin) => {
                    Resolved::Moved { now, pinned: pin }
                }
                _ => Resolved::Current(now),
            }),
            Err(e) => match ClientError::from(e) {
                ClientError::Fault(FilesFault::PathNotFound(_)) => Ok(Resolved::Missing { pinned }),
                other => Err(other),
            },
        }
    }

    /// The bytes a reference pinned: the path's version with that
    /// content (its current one, or one in its history), or the current
    /// bytes when it pinned none.
    pub async fn get_ref(
        &self,
        root: RootId,
        path: &str,
        pinned: Option<ContentId>,
    ) -> Result<Vec<u8>> {
        let path = parse(path)?;
        let Some(pin) = pinned else {
            return self
                .read(ReadRequest {
                    root_id: root,
                    path,
                    version: None,
                    range: None,
                })
                .await;
        };
        let history = self.service.history(root, path.clone()).await?;
        let version = history
            .iter()
            .rev()
            .find(|v| v.content.as_ref() == Some(&pin))
            .ok_or_else(|| {
                ClientError::Fault(FilesFault::invalid(format!("{path} never held {pin}")))
            })?;
        self.read(ReadRequest {
            root_id: root,
            path,
            version: Some(version.id),
            range: None,
        })
        .await
    }

    // ── In a browser ───────────────────────────────────────────────

    /// A read ticket for `path` and what is under it, for a byte URL an
    /// element that cannot send a header loads (`<video src>`).
    pub async fn ticket(&self, root: RootId, path: &str) -> Result<Ticket> {
        Ok(self.service.ticket(root, parse(path)?).await?)
    }

    /// A version's renditions (`None`: the current one), asking for any
    /// that are missing.
    pub async fn renditions(
        &self,
        root: RootId,
        path: &str,
        version: Option<VersionId>,
    ) -> Result<RenditionSet> {
        Ok(self.service.renditions(root, parse(path)?, version).await?)
    }

    // ── Live ───────────────────────────────────────────────────────

    /// Follow changes: `on_event` gets each one until it returns `false`
    /// or the stream ends. Subscribe before reading current state (the
    /// first event, [`FilesEvent::Attached`], says the subscription is
    /// live), then fold events in.
    pub async fn events(
        &self,
        root: Option<RootId>,
        mut on_event: impl FnMut(FilesEvent) -> bool,
    ) -> Result<()> {
        let (tx, mut rx) = vox::channel::<FilesEvent>();
        let listening = async move {
            loop {
                match rx.recv().await {
                    Ok(Some(frame)) => {
                        let mut owned = None;
                        let _ = frame.map(|e| owned = Some(e));
                        if let Some(event) = owned
                            && !on_event(event)
                        {
                            return Ok(());
                        }
                    }
                    Ok(None) => return Ok(()),
                    Err(e) => return Err(ClientError::Transport(format!("{e:?}"))),
                }
            }
        };
        drive(self.stream.events(root, tx), listening).await
    }
}

/// A subscription call and the loop reading it, finishing when the
/// reader does (the call itself never returns: dropping it unsubscribes).
async fn drive<C: Future, L: Future>(call: C, listen: L) -> L::Output {
    let call = std::pin::pin!(call);
    let listen = std::pin::pin!(listen);
    match select(call, listen).await {
        Either::Left((_, listen)) => listen.await,
        Either::Right((out, _)) => out,
    }
}

fn parse(path: &str) -> Result<RootPath> {
    RootPath::parse(path).map_err(|e| ClientError::Fault(FilesFault::BadPath(e)))
}

fn len_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
