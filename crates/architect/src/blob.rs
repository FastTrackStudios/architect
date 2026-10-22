//! The blob lane: resumable, sha256-verified transfer of a file into
//! place ([`ResumableFetch`]) and a content-addressed store
//! ([`BlobStore`], with the [`LocalFs`] backend).
//!
//! # `ResumableFetch`
//!
//! One file, `dest`, of a known size and (usually) a known sha256, pulled
//! from any [`Source`] that can start reading at an offset (a vox stream,
//! an HTTP `Range` request, a file on a mounted drive):
//!
//! - **Resumable.** Bytes land in `<dest>.part`, opened for append. A run
//!   that stops for any reason (an error, the network going away, the
//!   future being dropped to pause) leaves the `.part`; the next run hashes
//!   what is there and asks the source for the rest from that offset.
//! - **Verified.** The whole file is hashed (the resumed prefix plus the
//!   new bytes, in one pass). A wrong hash removes the `.part` and the
//!   fetch starts again from zero, up to [`ResumableFetch::attempts`]
//!   times (default 2), so a corrupt prefix left by an earlier run costs
//!   one retry, not a stuck download.
//! - **Atomic.** The verified `.part` is synced and renamed over `dest`;
//!   a reader never sees a torn file.
//! - **Sidecar identity.** A finished fetch writes `<dest>.sha256`
//!   (`"<size>:<hex>\n"`, the format Signal's content client uses). A
//!   later fetch of the same file with the same size and hash returns at
//!   once without reading the file or calling the source: kits sharing a
//!   pack fetch it once. A sidecar whose size no longer matches the file
//!   is ignored.
//! - **One transfer per file.** A second concurrent fetch of the same
//!   `dest` path fails with [`BlobError::Busy`] instead of appending into
//!   the same `.part`. The guard is per process and keyed by the path as
//!   given, so pass the same path for the same file.
//!
//! # `BlobStore`
//!
//! A content-addressed store: a blob's identity is its sha256. `put`
//! streams the body into a temporary file while hashing it, checks the
//! hash the caller expected (if any), and renames it into place; storing
//! the same bytes twice is a no-op. `get` returns a stream and its
//! length; `has` asks without reading. [`LocalFs`] keeps blobs at
//! `<root>/<hex[0..2]>/<hex>`.
//!
//! Native only for now. The wasm side (an OPFS file plus an `IndexedDB`
//! ledger, as Signal's web packs do) is a follow-up.
//!
//! Extracted from Signal's content client and pack downloads and the Task
//! server's attachment object store (STA-388).

use std::collections::HashSet;
use std::fmt;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use futures::future::BoxFuture;
use sha2::Digest as _;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};

/// A boxed byte stream: what a [`Source`] serves and a [`BlobStore`]
/// takes and returns.
pub type Reader = Pin<Box<dyn AsyncRead + Send>>;

/// Read/write buffer for one transfer (on the heap, so the futures stay
/// small).
const BUF: usize = 64 * 1024;

// ── Sha256 ──────────────────────────────────────────────────────────

/// A sha256 digest. Displays and parses as 64 lowercase hex digits
/// (parsing also takes uppercase).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256([u8; 32]);

impl Sha256 {
    /// The digest with these bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The sha256 of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(sha2::Sha256::digest(bytes).into())
    }

    /// 64 lowercase hex digits.
    #[must_use]
    pub fn to_hex(&self) -> String {
        use fmt::Write as _;
        let mut out = String::with_capacity(64);
        for b in self.0 {
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    /// Parse 64 hex digits.
    pub fn from_hex(hex: &str) -> Result<Self, BlobError> {
        let bad = || BlobError::BadHash(hex.to_owned());
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let mut out = [0u8; 32];
        for (slot, pair) in out.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
            let pair = std::str::from_utf8(pair).map_err(|_| bad())?;
            *slot = u8::from_str_radix(pair, 16).map_err(|_| bad())?;
        }
        Ok(Self(out))
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({})", self.to_hex())
    }
}

impl FromStr for Sha256 {
    type Err = BlobError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

// ── errors ──────────────────────────────────────────────────────────

/// Why a fetch or a store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    /// A local file could not be read, written or renamed.
    #[error("blob i/o at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The source failed to open or broke off mid-stream. What arrived is
    /// kept in the `.part`; the next fetch resumes from there.
    #[error("blob source: {0}")]
    Source(#[source] io::Error),
    /// The bytes do not hash to what was expected. The `.part` (or the
    /// store's temporary file) was removed.
    #[error("sha256 mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: Sha256,
        actual: Sha256,
    },
    /// The source ended before the expected size. The `.part` is kept;
    /// fetching again resumes.
    #[error("incomplete {path}: {have} of {expected} bytes")]
    Incomplete {
        path: PathBuf,
        have: u64,
        expected: u64,
    },
    /// The source sent more than the expected size. The `.part` was
    /// removed.
    #[error("{path}: the source sent more than the expected {expected} bytes")]
    TooLong { path: PathBuf, expected: u64 },
    /// Another fetch of the same file is in flight in this process.
    #[error("{0} is already being transferred")]
    Busy(PathBuf),
    /// The store has no blob with this hash.
    #[error("no blob {0}")]
    NotFound(Sha256),
    /// Not 64 hex digits.
    #[error("not a sha256 hex digest: {0:?}")]
    BadHash(String),
    /// A host-supplied relative path that would leave its root.
    #[error("refusing the path {0:?}: not a plain relative path")]
    BadPath(String),
}

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> BlobError + '_ {
    move |source| BlobError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// ── paths ───────────────────────────────────────────────────────────

/// `rel`, a `/`-separated path from a host or a manifest, joined under
/// `root`.
///
/// Refuses anything but plain names: an empty segment (`a//b`, a leading
/// `/`), `.`, `..`, or an absolute or prefixed segment, so the result
/// always stays inside `root` and a hostile host cannot place a file
/// elsewhere.
pub fn join_under(root: &Path, rel: &str) -> Result<PathBuf, BlobError> {
    let mut path = root.to_path_buf();
    for seg in rel.split('/') {
        let mut parts = Path::new(seg).components();
        match (parts.next(), parts.next()) {
            (Some(Component::Normal(name)), None) if name == seg => path.push(name),
            _ => return Err(BlobError::BadPath(rel.to_owned())),
        }
    }
    Ok(path)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// `<path>.part`: where a fetch of `path` stages its bytes.
#[must_use]
pub fn part_path(path: &Path) -> PathBuf {
    with_suffix(path, ".part")
}

/// `<path>.sha256`: the sidecar a finished fetch leaves beside `path`.
#[must_use]
pub fn sidecar_path(path: &Path) -> PathBuf {
    with_suffix(path, ".sha256")
}

/// The hash the sidecar beside `path` records, if the sidecar is there,
/// well formed, and its size still matches the file's.
#[must_use]
pub fn read_sidecar(path: &Path) -> Option<Sha256> {
    let size = std::fs::metadata(path).ok()?.len();
    let text = std::fs::read_to_string(sidecar_path(path)).ok()?;
    let (len, hex) = text.trim().split_once(':')?;
    if len.parse::<u64>().ok()? != size {
        return None;
    }
    Sha256::from_hex(hex).ok()
}

fn sidecar_body(size: u64, hash: &Sha256) -> String {
    format!("{size}:{hash}\n")
}

// ── one transfer per file ───────────────────────────────────────────

fn in_flight() -> &'static Mutex<HashSet<PathBuf>> {
    static SET: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    SET.get_or_init(Mutex::default)
}

/// A file's in-flight slot, released however the fetch ends (including a
/// dropped future).
struct Slot(PathBuf);

impl Slot {
    fn take(path: &Path) -> Option<Self> {
        let fresh = in_flight()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_path_buf());
        fresh.then(|| Self(path.to_path_buf()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        in_flight()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

// ── Source ──────────────────────────────────────────────────────────

/// Where a [`ResumableFetch`] reads from: the file's bytes from `offset`
/// to its end. An HTTP source sends `Range: bytes=<offset>-`; a vox
/// source passes the offset to its read call.
pub trait Source: Send + Sync {
    /// The bytes from `offset` to the end of the file.
    fn read_from(&self, offset: u64) -> BoxFuture<'_, io::Result<Reader>>;
}

// ── ResumableFetch ──────────────────────────────────────────────────

/// What a finished [`ResumableFetch::run`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    /// The file, in place.
    pub path: PathBuf,
    /// Its sha256 (also in the sidecar).
    pub sha256: Sha256,
    /// `true` when the file was already here (size and sidecar matched)
    /// and the source was not called.
    pub skipped: bool,
    /// Bytes the last attempt found in the `.part` and did not fetch again.
    pub resumed_from: u64,
}

/// Fetch one file into place: resumable, verified, atomic, one at a time.
/// See the [module docs](self).
#[derive(Clone, Debug)]
pub struct ResumableFetch {
    dest: PathBuf,
    size: u64,
    sha256: Option<Sha256>,
    attempts: u32,
}

impl ResumableFetch {
    /// Fetch into `dest`, a file of `size` bytes. Set the expected hash
    /// with [`Self::sha256`] whenever the source publishes one.
    pub fn new(dest: impl Into<PathBuf>, size: u64) -> Self {
        Self {
            dest: dest.into(),
            size,
            sha256: None,
            attempts: 2,
        }
    }

    /// The hash the file must have. Without one the file is still hashed
    /// (for the sidecar) but any content of the right size is accepted.
    #[must_use]
    pub const fn sha256(mut self, hash: Sha256) -> Self {
        self.sha256 = Some(hash);
        self
    }

    /// How many times one [`Self::run`] tries after a hash mismatch
    /// (at least 1; default 2).
    #[must_use]
    pub fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    /// The destination file.
    #[must_use]
    pub fn dest(&self) -> &Path {
        &self.dest
    }

    /// The expected size.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// `<dest>.part`.
    #[must_use]
    pub fn part_path(&self) -> PathBuf {
        part_path(&self.dest)
    }

    /// `<dest>.sha256`.
    #[must_use]
    pub fn sidecar_path(&self) -> PathBuf {
        sidecar_path(&self.dest)
    }

    /// Bytes waiting in the `.part` (0 when there is none).
    #[must_use]
    pub fn partial_len(&self) -> u64 {
        std::fs::metadata(self.part_path()).map_or(0, |m| m.len())
    }

    /// Whether the file is already here, whole: the right size, and a
    /// sidecar that matches it and (when one is set) the expected hash.
    /// Reads the sidecar, never the file.
    #[must_use]
    pub fn is_here(&self) -> bool {
        self.here().is_some()
    }

    fn here(&self) -> Option<Sha256> {
        let hash = read_sidecar(&self.dest)?;
        let size_ok = std::fs::metadata(&self.dest).ok()?.len() == self.size;
        let hash_ok = self.sha256.is_none_or(|want| want == hash);
        (size_ok && hash_ok).then_some(hash)
    }

    fn skipped(&self, sha256: Sha256) -> Fetched {
        Fetched {
            path: self.dest.clone(),
            sha256,
            skipped: true,
            resumed_from: 0,
        }
    }

    /// Fetch the file from `source`, resuming any `.part`, and put it in
    /// place. `progress` gets the bytes in the `.part` after every write.
    ///
    /// Dropping the returned future pauses: the `.part` stays and the
    /// next run resumes from it.
    pub async fn run<S, P>(&self, source: &S, mut progress: P) -> Result<Fetched, BlobError>
    where
        S: Source + ?Sized,
        P: FnMut(u64) + Send,
    {
        if let Some(hash) = self.here() {
            return Ok(self.skipped(hash));
        }
        let _slot = Slot::take(&self.dest).ok_or_else(|| BlobError::Busy(self.dest.clone()))?;
        // Another fetch may have finished between the check and the slot.
        if let Some(hash) = self.here() {
            return Ok(self.skipped(hash));
        }
        let mut tries = 1;
        loop {
            match self.attempt(source, &mut progress).await {
                Err(BlobError::HashMismatch { .. }) if tries < self.attempts => {
                    tries = tries.saturating_add(1);
                }
                other => return other,
            }
        }
    }

    /// One pass: hash the `.part`, fetch the rest, verify, rename.
    async fn attempt<S, P>(&self, source: &S, progress: &mut P) -> Result<Fetched, BlobError>
    where
        S: Source + ?Sized,
        P: FnMut(u64) + Send,
    {
        if let Some(dir) = self.dest.parent().filter(|d| !d.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(dir).await.map_err(io_at(dir))?;
        }
        let part = self.part_path();
        let (mut hasher, start) = self.hash_part(&part).await?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part)
            .map_err(io_at(&part))?;
        let mut done = start;
        if done < self.size {
            let reader = source.read_from(start).await.map_err(BlobError::Source)?;
            done = self
                .append(reader, &mut file, &mut hasher, done, progress)
                .await?;
        }
        file.sync_all().map_err(io_at(&part))?;
        drop(file);
        if done < self.size {
            return Err(BlobError::Incomplete {
                path: self.dest.clone(),
                have: done,
                expected: self.size,
            });
        }
        let actual = Sha256(hasher.finalize().into());
        if let Some(expected) = self.sha256.filter(|want| *want != actual) {
            let _ = std::fs::remove_file(&part);
            return Err(BlobError::HashMismatch {
                path: self.dest.clone(),
                expected,
                actual,
            });
        }
        self.commit(&part, actual)?;
        Ok(Fetched {
            path: self.dest.clone(),
            sha256: actual,
            skipped: false,
            resumed_from: start,
        })
    }

    /// The hasher fed with the `.part` so far, and its length. A `.part`
    /// longer than the file is stale and starts again.
    async fn hash_part(&self, part: &Path) -> Result<(sha2::Sha256, u64), BlobError> {
        let mut hasher = sha2::Sha256::new();
        let mut file = match tokio::fs::File::open(part).await {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((hasher, 0)),
            Err(e) => return Err(io_at(part)(e)),
        };
        let len = file.metadata().await.map_err(io_at(part))?.len();
        if len > self.size {
            drop(file);
            tokio::fs::remove_file(part).await.map_err(io_at(part))?;
            return Ok((hasher, 0));
        }
        let mut buf = vec![0u8; BUF];
        loop {
            let n = file.read(&mut buf).await.map_err(io_at(part))?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
                break;
            };
            hasher.update(chunk);
        }
        Ok((hasher, len))
    }

    /// Copy `reader` onto the end of the `.part`, hashing as it goes.
    /// Writes are synchronous so nothing is in flight when the future is
    /// dropped: the `.part` length is always what arrived.
    async fn append<P: FnMut(u64) + Send>(
        &self,
        mut reader: Reader,
        file: &mut std::fs::File,
        hasher: &mut sha2::Sha256,
        mut done: u64,
        progress: &mut P,
    ) -> Result<u64, BlobError> {
        let part = self.part_path();
        let mut buf = vec![0u8; BUF];
        loop {
            let n = reader.read(&mut buf).await.map_err(BlobError::Source)?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
                return Ok(done);
            };
            let next = u64::try_from(n)
                .ok()
                .and_then(|n| done.checked_add(n))
                .filter(|next| *next <= self.size);
            let Some(next) = next else {
                let _ = std::fs::remove_file(&part);
                return Err(BlobError::TooLong {
                    path: self.dest.clone(),
                    expected: self.size,
                });
            };
            file.write_all(chunk).map_err(io_at(&part))?;
            hasher.update(chunk);
            done = next;
            progress(done);
        }
    }

    /// The verified `.part` into place, then its sidecar. The old sidecar
    /// goes first, so a crash in between leaves a file with no sidecar
    /// (fetched again), never a sidecar vouching for other bytes.
    fn commit(&self, part: &Path, hash: Sha256) -> Result<(), BlobError> {
        let sidecar = self.sidecar_path();
        if let Err(e) = std::fs::remove_file(&sidecar)
            && e.kind() != io::ErrorKind::NotFound
        {
            return Err(io_at(&sidecar)(e));
        }
        std::fs::rename(part, &self.dest).map_err(io_at(&self.dest))?;
        std::fs::write(&sidecar, sidecar_body(self.size, &hash)).map_err(io_at(&sidecar))
    }
}

// ── BlobStore ───────────────────────────────────────────────────────

/// A stored blob being read.
pub struct Blob {
    /// Its length in bytes.
    pub len: u64,
    /// Its bytes.
    pub reader: Reader,
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// A content-addressed blob store: a blob's identity is its sha256, and
/// storing the same bytes twice is a no-op. Object-safe, so an app can
/// hold an `Arc<dyn BlobStore>` and swap backends.
pub trait BlobStore: Send + Sync {
    /// Store `body` and return its hash. With `expected`, the bytes must
    /// hash to it (else [`BlobError::HashMismatch`] and nothing is
    /// stored), and a blob already present under it is not read again.
    fn put(
        &self,
        expected: Option<Sha256>,
        body: Reader,
    ) -> BoxFuture<'_, Result<Sha256, BlobError>>;

    /// The blob's bytes as a stream, and its length.
    fn get(&self, hash: Sha256) -> BoxFuture<'_, Result<Blob, BlobError>>;

    /// Whether the store holds the blob.
    fn has(&self, hash: Sha256) -> BoxFuture<'_, Result<bool, BlobError>>;
}

/// [`BlobStore`] on the local filesystem: `<root>/<hex[0..2]>/<hex>`,
/// written through `<root>/.tmp/` and renamed into place.
#[derive(Clone, Debug)]
pub struct LocalFs {
    root: PathBuf,
}

impl LocalFs {
    /// A store under `root` (created on the first put).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The store's root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the blob `hash` lives.
    #[must_use]
    pub fn path_for(&self, hash: &Sha256) -> PathBuf {
        let hex = hash.to_hex();
        let prefix = hex.get(..2).unwrap_or_default().to_owned();
        self.root.join(prefix).join(hex)
    }

    /// Store `bytes` (see [`BlobStore::put`]).
    pub async fn put_bytes(&self, bytes: &[u8]) -> Result<Sha256, BlobError> {
        let body: Reader = Box::pin(std::io::Cursor::new(bytes.to_vec()));
        self.put_reader(Some(Sha256::of(bytes)), body).await
    }

    fn temp_path(&self) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        self.root
            .join(".tmp")
            .join(format!("{}-{n}.part", std::process::id()))
    }

    async fn put_reader(
        &self,
        expected: Option<Sha256>,
        body: Reader,
    ) -> Result<Sha256, BlobError> {
        if let Some(hash) = expected
            && self.has_blob(hash).await?
        {
            return Ok(hash);
        }
        let tmp = self.temp_path();
        let result = self.stage(&tmp, expected, body).await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }

    /// Stream `body` into `tmp`, verify, and rename it into place.
    async fn stage(
        &self,
        tmp: &Path,
        expected: Option<Sha256>,
        mut body: Reader,
    ) -> Result<Sha256, BlobError> {
        if let Some(dir) = tmp.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(io_at(dir))?;
        }
        let mut file = tokio::fs::File::create(tmp).await.map_err(io_at(tmp))?;
        let mut hasher = sha2::Sha256::new();
        let mut buf = vec![0u8; BUF];
        loop {
            let n = body.read(&mut buf).await.map_err(BlobError::Source)?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
                break;
            };
            file.write_all(chunk).await.map_err(io_at(tmp))?;
            hasher.update(chunk);
        }
        file.sync_all().await.map_err(io_at(tmp))?;
        drop(file);
        let actual = Sha256(hasher.finalize().into());
        if let Some(expected) = expected.filter(|want| *want != actual) {
            return Err(BlobError::HashMismatch {
                path: tmp.to_path_buf(),
                expected,
                actual,
            });
        }
        let dest = self.path_for(&actual);
        if self.has_blob(actual).await? {
            let _ = tokio::fs::remove_file(tmp).await;
            return Ok(actual);
        }
        if let Some(dir) = dest.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(io_at(dir))?;
        }
        tokio::fs::rename(tmp, &dest).await.map_err(io_at(&dest))?;
        Ok(actual)
    }

    async fn get_blob(&self, hash: Sha256) -> Result<Blob, BlobError> {
        let path = self.path_for(&hash);
        let file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(BlobError::NotFound(hash)),
            Err(e) => return Err(io_at(&path)(e)),
        };
        let len = file.metadata().await.map_err(io_at(&path))?.len();
        Ok(Blob {
            len,
            reader: Box::pin(file),
        })
    }

    async fn has_blob(&self, hash: Sha256) -> Result<bool, BlobError> {
        let path = self.path_for(&hash);
        tokio::fs::try_exists(&path).await.map_err(io_at(&path))
    }
}

impl BlobStore for LocalFs {
    fn put(
        &self,
        expected: Option<Sha256>,
        body: Reader,
    ) -> BoxFuture<'_, Result<Sha256, BlobError>> {
        Box::pin(self.put_reader(expected, body))
    }

    fn get(&self, hash: Sha256) -> BoxFuture<'_, Result<Blob, BlobError>> {
        Box::pin(self.get_blob(hash))
    }

    fn has(&self, hash: Sha256) -> BoxFuture<'_, Result<bool, BlobError>> {
        Box::pin(self.has_blob(hash))
    }
}
