//! Blobs by hash from somewhere else, on demand: [`Remote`] (anything that
//! serves a blob's bytes from an offset, given its hash) and [`Fetcher`]
//! (a [`LocalFs`] that fills itself from its remotes when a blob it does
//! not hold is asked for).
//!
//! - **One transfer per blob.** However many callers open or fetch the
//!   same hash, one transfer runs; each watches its [`Progress`].
//! - **Resumable.** A transfer is a [`ResumableFetch`] into
//!   [`LocalFs::partial_path`]: a dropped link leaves the `.part`, and the
//!   next fetch (from the same remote or another) asks only for the rest.
//!   A transfer that fails falls through to the next remote, resuming.
//! - **Streamed.** [`Fetcher::open_at`] hands back a reader at once that
//!   yields bytes as they land, so playback starts before the end
//!   arrives. Streamed bytes are checked only when the whole blob is in:
//!   a transfer that turns out corrupt and starts again ends the stream
//!   with an `InvalidData` error rather than serving other bytes.
//! - **Verified, then in place.** The finished blob hashes to its name
//!   and moves to [`LocalFs::path_for`]; from then on it is a local blob.
//!
//! A transfer runs on its own task, so dropping the future that started
//! it does not stop it: a blob asked for is fetched to the end.

use std::collections::HashMap;
use std::io::{self, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use futures::future::BoxFuture;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
use tokio::sync::{mpsc, watch};

use super::stream_reader::StreamReader;
use super::{BUF, Blob, BlobError, LocalFs, Reader, ResumableFetch, Sha256, Source};

// ── Remote ──────────────────────────────────────────────────────────

/// Somewhere blobs can be read from by hash: a peer over vox or iroh
/// ([`super::VoxRemote`]), a server's HTTP route ([`super::HttpRemote`]),
/// another store on a mounted drive ([`LocalFs`] is one).
pub trait Remote: Send + Sync {
    /// The blob's length, or `None` when this remote does not have it.
    fn size(&self, hash: Sha256) -> BoxFuture<'_, io::Result<Option<u64>>>;

    /// The blob's bytes from `offset` to its end.
    fn read_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, io::Result<Reader>>;
}

impl Remote for LocalFs {
    fn size(&self, hash: Sha256) -> BoxFuture<'_, io::Result<Option<u64>>> {
        Box::pin(async move {
            match tokio::fs::metadata(self.path_for(&hash)).await {
                Ok(meta) => Ok(Some(meta.len())),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    fn read_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        Box::pin(async move {
            self.get_blob_from(hash, offset)
                .await
                .map(|blob| blob.reader)
                .map_err(io::Error::other)
        })
    }
}

/// One blob on a [`Remote`], as the [`Source`] a [`ResumableFetch`] reads.
pub struct HashSource<'a> {
    pub remote: &'a dyn Remote,
    pub hash: Sha256,
}

impl Source for HashSource<'_> {
    fn read_from(&self, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        self.remote.read_from(self.hash, offset)
    }
}

// ── Progress ────────────────────────────────────────────────────────

/// Where one blob's transfer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Looking for a remote that has it.
    Finding,
    /// `have` of `size` bytes are in the `.part`. `restarts` counts the
    /// times a corrupt transfer started again from zero.
    Fetching { have: u64, size: u64, restarts: u32 },
    /// In the store.
    Done,
    /// No remote has it.
    Missing,
    /// Every remote that has it failed; the `.part` is kept for the next
    /// try.
    Failed(String),
    /// The store's budget has no room for it ([`super::StoreBudget`]):
    /// deferred, nothing fetched. Fetching again once there is room
    /// resumes.
    NoRoom(super::NoRoom),
}

// ── Fetcher ─────────────────────────────────────────────────────────

struct Inner {
    store: LocalFs,
    remotes: RwLock<Vec<Arc<dyn Remote>>>,
    running: Mutex<HashMap<Sha256, watch::Receiver<Progress>>>,
}

/// A [`LocalFs`] that fetches the blobs it lacks from its [`Remote`]s,
/// tried in order. Cheap to clone. See the [module docs](self).
#[derive(Clone)]
pub struct Fetcher {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Fetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetcher")
            .field("store", &self.inner.store)
            .finish_non_exhaustive()
    }
}

impl Fetcher {
    /// Fetch into `store`. Add remotes with [`Self::with_remote`].
    #[must_use]
    pub fn new(store: LocalFs) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                remotes: RwLock::new(Vec::new()),
                running: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Also fetch from `remote`, after the ones already added.
    #[must_use]
    pub fn with_remote(self, remote: Arc<dyn Remote>) -> Self {
        self.add_remote(remote);
        self
    }

    /// Also fetch from `remote`, after the ones already added.
    pub fn add_remote(&self, remote: Arc<dyn Remote>) {
        self.inner
            .remotes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(remote);
    }

    /// Replace the remotes (a peer reconnected on a new link, a server's
    /// address changed). Transfers already running keep theirs.
    pub fn set_remotes(&self, remotes: Vec<Arc<dyn Remote>>) {
        *self
            .inner
            .remotes
            .write()
            .unwrap_or_else(PoisonError::into_inner) = remotes;
    }

    /// The store it fetches into.
    #[must_use]
    pub fn store(&self) -> &LocalFs {
        &self.inner.store
    }

    /// The running transfer of `hash`, if there is one.
    #[must_use]
    pub fn progress(&self, hash: Sha256) -> Option<watch::Receiver<Progress>> {
        self.inner
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&hash)
            .cloned()
    }

    /// Have `hash` in the store: at once when it is here, else fetched
    /// whole from the first remote that has it (joining a transfer
    /// already running).
    ///
    /// # Errors
    ///
    /// [`BlobError::NotFound`] when no remote has it,
    /// [`BlobError::Unavailable`] when every one that has it failed (the
    /// `.part` is kept; fetching again resumes).
    pub async fn fetch(&self, hash: Sha256) -> Result<(), BlobError> {
        if self.inner.store.has_blob(hash).await? {
            return Ok(());
        }
        let mut rx = self.start(hash);
        loop {
            let now = rx.borrow_and_update().clone();
            if let Some(end) = finished(hash, &now) {
                return end;
            }
            if rx.changed().await.is_err() {
                return self.ended(hash).await;
            }
        }
    }

    /// The blob's bytes from the start; see [`Self::open_at`].
    ///
    /// # Errors
    ///
    /// As [`Self::open_at`].
    pub async fn open(&self, hash: Sha256) -> Result<Blob, BlobError> {
        self.open_at(hash, 0).await
    }

    /// The blob's bytes from `offset` (a player seeking). A blob in the
    /// store is read from disk; one that is not starts (or joins) its
    /// transfer and the reader yields bytes as they arrive. `len` is the
    /// bytes from `offset` to the end.
    ///
    /// # Errors
    ///
    /// No remote has it ([`BlobError::NotFound`]), or none could start
    /// sending it ([`BlobError::Unavailable`]). A transfer failing after
    /// the reader is handed back is a read error.
    pub async fn open_at(&self, hash: Sha256, offset: u64) -> Result<Blob, BlobError> {
        if self.inner.store.has_blob(hash).await? {
            return self.inner.store.get_blob_from(hash, offset).await;
        }
        let mut rx = self.start(hash);
        let size = loop {
            let now = rx.borrow_and_update().clone();
            if let Progress::Fetching { size, .. } = now {
                break size;
            }
            if let Some(end) = finished(hash, &now) {
                end?;
                return self.inner.store.get_blob_from(hash, offset).await;
            }
            if rx.changed().await.is_err() {
                self.ended(hash).await?;
                return self.inner.store.get_blob_from(hash, offset).await;
            }
        };
        let offset = offset.min(size);
        let (tx, chunks) = mpsc::channel(4);
        tokio::spawn(tail(self.inner.store.clone(), hash, offset, rx, tx));
        Ok(Blob {
            len: size.saturating_sub(offset),
            reader: Box::pin(StreamReader::new(stream_of(chunks))),
        })
    }

    /// A transfer's channel closed without a final state (its task
    /// panicked or the runtime is going away): whatever the store says.
    async fn ended(&self, hash: Sha256) -> Result<(), BlobError> {
        if self.inner.store.has_blob(hash).await? {
            Ok(())
        } else {
            Err(BlobError::Unavailable {
                hash,
                reason: "the transfer stopped".to_owned(),
            })
        }
    }

    /// The transfer of `hash`, started if none is running.
    fn start(&self, hash: Sha256) -> watch::Receiver<Progress> {
        let mut running = self
            .inner
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(rx) = running.get(&hash) {
            return rx.clone();
        }
        let (tx, rx) = watch::channel(Progress::Finding);
        running.insert(hash, rx.clone());
        drop(running);
        let inner = self.inner.clone();
        let span = tracing::info_span!(
            "blob.fetch",
            blob = %hash,
            outcome = tracing::field::Empty,
            failures = tracing::field::Empty,
            // Set when the fetch is deferred for space (the wide event).
            store.need = tracing::field::Empty,
            store.used = tracing::field::Empty,
            store.max = tracing::field::Empty,
            fs.free = tracing::field::Empty,
            fs.keep_free = tracing::field::Empty,
        );
        let task = async move {
            let end = transfer(&inner, hash, &tx).await;
            let span = tracing::Span::current();
            span.record("outcome", tracing::field::debug(&end));
            // Final state first: whoever joins before the entry goes
            // sees how it ended.
            tx.send_replace(end);
            inner
                .running
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&hash);
        };
        tokio::spawn(tracing::Instrument::instrument(task, span));
        rx
    }
}

/// The result a final [`Progress`] stands for; `None` while running.
fn finished(hash: Sha256, progress: &Progress) -> Option<Result<(), BlobError>> {
    match progress {
        Progress::Finding | Progress::Fetching { .. } => None,
        Progress::Done => Some(Ok(())),
        Progress::Missing => Some(Err(BlobError::NotFound(hash))),
        Progress::Failed(reason) => Some(Err(BlobError::Unavailable {
            hash,
            reason: reason.clone(),
        })),
        Progress::NoRoom(no) => Some(Err(BlobError::NoRoom(no.clone()))),
    }
}

/// One blob from the first remote that has it and can send it whole.
async fn transfer(inner: &Inner, hash: Sha256, tx: &watch::Sender<Progress>) -> Progress {
    // A transfer that just finished may have landed it.
    if inner.store.has_blob(hash).await.unwrap_or(false) {
        return Progress::Done;
    }
    let remotes = inner
        .remotes
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mut failures = Vec::new();
    let mut restarts = 0u32;
    for remote in remotes {
        let size = match remote.size(hash).await {
            Ok(Some(size)) => size,
            Ok(None) => continue,
            Err(e) => {
                failures.push(e.to_string());
                continue;
            }
        };
        match fetch_from(inner, hash, size, remote.as_ref(), tx, &mut restarts).await {
            Ok(()) => return Progress::Done,
            // No other remote changes that: deferred until there is room.
            Err(BlobError::NoRoom(no)) => {
                let span = tracing::Span::current();
                span.record("store.need", no.need);
                span.record("store.used", no.used.saturating_add(no.reserved));
                span.record("store.max", no.max);
                if let Some(free) = no.free {
                    span.record("fs.free", free);
                }
                span.record("fs.keep_free", no.keep_free);
                return Progress::NoRoom(no);
            }
            Err(e) => failures.push(e.to_string()),
        }
    }
    if failures.is_empty() {
        Progress::Missing
    } else {
        let failures = failures.join("; ");
        tracing::Span::current().record("failures", failures.as_str());
        Progress::Failed(failures)
    }
}

/// Resume (or start) the `.part` from `remote`, then move it into place.
async fn fetch_from(
    inner: &Inner,
    hash: Sha256,
    size: u64,
    remote: &dyn Remote,
    tx: &watch::Sender<Progress>,
    restarts: &mut u32,
) -> Result<(), BlobError> {
    let staged = inner.store.partial_path(&hash);
    let job = ResumableFetch::new(&staged, size).sha256(hash);
    let before = job.partial_len();
    // The bytes still to come, reserved before any is asked for.
    let mut reservation = inner
        .store
        .reserve(size.saturating_sub(before))
        .map_err(BlobError::NoRoom)?;
    let mut last = before;
    tx.send_replace(Progress::Fetching {
        have: last,
        size,
        restarts: *restarts,
    });
    let ran = job
        .run(&HashSource { remote, hash }, |have| {
            if have < last {
                *restarts = restarts.saturating_add(1);
            }
            last = have;
            tx.send_replace(Progress::Fetching {
                have,
                size,
                restarts: *restarts,
            });
        })
        .await;
    // What the `.part` grew (or shrank) by is now the store's.
    let now = if ran.is_ok() { size } else { job.partial_len() };
    if let Some(r) = reservation.as_mut() {
        r.wrote(now.saturating_sub(before));
    }
    if let Some(b) = inner.store.budget() {
        b.removed(before.saturating_sub(now));
    }
    ran?;
    let dest = inner.store.path_for(&hash);
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(super::io_at(dir))?;
    }
    tokio::fs::rename(&staged, &dest)
        .await
        .map_err(super::io_at(&dest))?;
    let _ = tokio::fs::remove_file(super::sidecar_path(&staged)).await;
    Ok(())
}

// ── streaming a blob that is still arriving ─────────────────────────

fn stream_of(
    mut rx: mpsc::Receiver<io::Result<Vec<u8>>>,
) -> impl futures::Stream<Item = io::Result<Vec<u8>>> + Unpin {
    Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)))
}

/// The paths a blob's bytes may be at while it arrives and once it is in.
fn arriving_paths(store: &LocalFs, hash: &Sha256) -> [PathBuf; 3] {
    let staged = store.partial_path(hash);
    [super::part_path(&staged), staged, store.path_for(hash)]
}

/// Send the blob from `offset` as it lands, until its end or until the
/// reader goes away.
async fn tail(
    store: LocalFs,
    hash: Sha256,
    offset: u64,
    mut rx: watch::Receiver<Progress>,
    tx: mpsc::Sender<io::Result<Vec<u8>>>,
) {
    if let Err(e) = tail_inner(&store, hash, offset, &mut rx, &tx).await {
        let _ = tx.send(Err(e)).await;
    }
}

async fn tail_inner(
    store: &LocalFs,
    hash: Sha256,
    mut pos: u64,
    rx: &mut watch::Receiver<Progress>,
    tx: &mpsc::Sender<io::Result<Vec<u8>>>,
) -> io::Result<()> {
    let mut file: Option<tokio::fs::File> = None;
    let mut first_restarts = None;
    loop {
        let now = rx.borrow_and_update().clone();
        // How far the bytes go, and whether that is the end.
        let (avail, size) = match now {
            Progress::Fetching {
                have,
                size,
                restarts,
            } => {
                let first = *first_restarts.get_or_insert(restarts);
                if restarts != first {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("blob {hash}: the transfer was corrupt and started again"),
                    ));
                }
                (have, size)
            }
            Progress::Done => (u64::MAX, u64::MAX),
            Progress::Finding => (0, u64::MAX),
            Progress::Missing => return Err(io::Error::from(io::ErrorKind::NotFound)),
            Progress::Failed(reason) => {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, reason));
            }
            Progress::NoRoom(no) => {
                return Err(io::Error::new(io::ErrorKind::StorageFull, no.to_string()));
            }
        };
        if pos < avail {
            let f = match file.as_mut() {
                Some(f) => f,
                None => file.insert(open_arriving(store, &hash, pos).await?),
            };
            let left = avail.saturating_sub(pos);
            let want = usize::try_from(left).map_or(BUF, |left| left.min(BUF));
            let mut buf = vec![0u8; want];
            let n = f.read(&mut buf).await?;
            if n == 0 {
                if matches!(now, Progress::Done) {
                    return Ok(());
                }
            } else {
                buf.truncate(n);
                pos = pos.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
                if tx.send(Ok(buf)).await.is_err() {
                    return Ok(());
                }
                continue;
            }
        } else if pos >= size {
            return Ok(());
        }
        if rx.changed().await.is_err() {
            // The transfer ended and its final state was read above;
            // one more pass reads what remains.
            if !matches!(*rx.borrow(), Progress::Done) {
                return Err(io::Error::other(format!(
                    "blob {hash}: the transfer stopped"
                )));
            }
        }
    }
}

/// The arriving (or arrived) file, positioned at `pos`.
async fn open_arriving(store: &LocalFs, hash: &Sha256, pos: u64) -> io::Result<tokio::fs::File> {
    for path in arriving_paths(store, hash) {
        match tokio::fs::File::open(&path).await {
            Ok(mut f) => {
                f.seek(SeekFrom::Start(pos)).await?;
                return Ok(f);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("blob {hash}: not arriving"),
    ))
}
