//! A link's files from wherever they are held (#490).
//!
//! The node that answers a share link need not hold what the link
//! serves: its holding rule (#904) leaves a project's multitracks on the
//! storage server, and a file may be in the bucket alone. [`Holders`]
//! answers one file, named by its sha-256, from the nearest place that
//! has it, in this order:
//!
//! 1. **Here.** A working file or the node's store: streamed from disk,
//!    ranges honoured.
//! 2. **The bucket.** A `302` to a presigned `GET` of that one object
//!    ([`SIGNED_FOR`]). The browser reads the bytes, ranges too, from the
//!    bucket itself; none pass through this node. The URL reads that one
//!    blob until it runs out: it lists nothing, names no other object and
//!    writes nothing, and the link's own URL (which is what a guest keeps)
//!    is checked again on every request, so a revoke or a lowered layer
//!    stops the next redirect.
//! 3. **A peer.** A holder this node is connected to (the storage
//!    server) sends the range over the nodes' own lane and this node
//!    passes it on ([`architect::blob::Fetcher::relay`]). Nothing is
//!    written here: the holding rule holds. At most [`RELAYS`] at once;
//!    one more waits [`RELAY_WAIT`] for a place, then is told to come
//!    back (`503`, `Retry-After`). A holder that does not start sending
//!    within [`FIRST_BYTE`] is a `504`. Once bytes flow the listener gets
//!    them at the holder's pace: a slow holder is a slow stream, never a
//!    held-up node. A file is relayed a window at a time ([`WINDOW`]),
//!    so a listener who leaves (a seek, a closed tab) costs the holder
//!    at most the rest of one window.
//!
//! No size limit in any path: nothing is buffered whole.
//!
//! It also knows which of a vault's attachments are not on this node's
//! disk ([`Holders::elsewhere`]): the vault root's manifest lists them
//! (`ARCHITECT_VAULT_FILES`), so a link names and serves them as it does
//! the ones that are.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use architect::blob::{S3Store, Sha256};
use architect::http::axum::body::Body;
use architect::http::axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use architect::http::axum::response::{IntoResponse, Response};
use architect_files::sync::{Engine, manifest};
use tokio::io::AsyncReadExt as _;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

/// How long a presigned bucket link reads its one object.
pub const SIGNED_FOR: Duration = Duration::from_secs(600);
/// How many files are relayed from peers at once: half of what one
/// connection between two nodes carries (vox closes a connection past 64
/// requests in flight), the rest left to the nodes' own sync.
pub const RELAYS: usize = 32;
/// How long a relay waits for a place before it is told to come back.
pub const RELAY_WAIT: Duration = Duration::from_secs(10);
/// How long a holder has to start sending.
pub const FIRST_BYTE: Duration = Duration::from_secs(15);
/// How much of a file one read of the holder asks for.
pub const WINDOW: u64 = 4 << 20;
/// How long the bucket is trusted to still hold what it held.
const BUCKET_KNOWN: Duration = Duration::from_secs(300);

/// A file by its content: what a holder is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileAt {
    pub hash: Sha256,
    pub size: u64,
}

impl FileAt {
    /// From a manifest's version: its content (hex) and size.
    #[must_use]
    pub fn of(content: Option<&str>, size: u64) -> Option<Self> {
        Some(Self {
            hash: Sha256::from_hex(content?).ok()?,
            size,
        })
    }
}

/// Where an answer's bytes came from: `x-architect-from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum From {
    Here,
    Bucket,
    Peer,
}

impl From {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Here => "here",
            Self::Bucket => "bucket",
            Self::Peer => "peer",
        }
    }
}

struct Inner {
    engine: Engine,
    bucket: Option<S3Store>,
    relays: Arc<Semaphore>,
    /// What the bucket was last seen to hold.
    in_bucket: Mutex<HashMap<Sha256, Instant>>,
    /// Each vault's attachments root (`<org>/Vaults/<vault>`).
    vault_roots: HashMap<String, Uuid>,
    /// Each vault's attachments that are not on this disk.
    elsewhere: RwLock<HashMap<String, Arc<BTreeMap<String, FileAt>>>>,
}

/// The files a link serves, from wherever they are. Cheap to clone.
#[derive(Clone)]
pub struct Holders(Arc<Inner>);

impl Holders {
    /// Over `engine` (its store, its working files, the peers its
    /// fetcher reads from) and the node's `bucket`, with each vault's
    /// attachments root.
    #[must_use]
    pub fn new(
        engine: Engine,
        bucket: Option<S3Store>,
        vault_roots: HashMap<String, Uuid>,
    ) -> Self {
        Self(Arc::new(Inner {
            engine,
            bucket,
            relays: Arc::new(Semaphore::new(RELAYS)),
            in_bucket: Mutex::new(HashMap::new()),
            vault_roots,
            elsewhere: RwLock::new(HashMap::new()),
        }))
    }

    /// The attachments of `vault` that are not in its folder here, by
    /// vault path, as of the last [`Self::refresh`].
    #[must_use]
    pub fn elsewhere(&self, vault: &str) -> Arc<BTreeMap<String, FileAt>> {
        self.0
            .elsewhere
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(vault)
            .cloned()
            .unwrap_or_default()
    }

    /// Read each vault's attachments root again; the vaults whose set of
    /// files held elsewhere changed.
    pub async fn refresh(&self) -> Vec<String> {
        let mut changed = Vec::new();
        for (vault, doc_id) in &self.0.vault_roots {
            let (Ok(tree), Some(dir)) = (
                self.0.engine.tree(*doc_id).await,
                self.0.engine.working_dir(*doc_id),
            ) else {
                continue;
            };
            let live = tree.live();
            let found = tokio::task::spawn_blocking(move || {
                live.into_iter()
                    .filter(|(path, _)| !dir.join(path).is_file())
                    .filter_map(|(path, v)| Some((path, FileAt::of(v.content.as_deref(), v.size)?)))
                    .collect::<BTreeMap<String, FileAt>>()
            })
            .await
            .unwrap_or_default();
            if *self.elsewhere(vault) != found {
                self.0
                    .elsewhere
                    .write()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(vault.clone(), Arc::new(found));
                changed.push(vault.clone());
            }
        }
        changed
    }

    /// The live files of the root at `place` (`<org>/Projects/<name>`),
    /// by root-relative path; `None` when this node does not know it.
    pub async fn root_files(&self, place: &str) -> Option<BTreeMap<String, FileAt>> {
        let doc_id = manifest::doc_id_for(place);
        if !self.0.engine.knows(doc_id) {
            return None;
        }
        let tree = self.0.engine.tree(doc_id).await.ok()?;
        Some(
            tree.live()
                .into_iter()
                .filter_map(|(path, v)| Some((path, FileAt::of(v.content.as_deref(), v.size)?)))
                .collect(),
        )
    }

    /// Save `bytes` as the new version of `path` in the root at `place`.
    ///
    /// # Errors
    ///
    /// This node does not know the root, or the engine refused the save.
    pub async fn save(&self, place: &str, path: &str, bytes: &[u8]) -> Result<FileAt, String> {
        let doc_id = manifest::doc_id_for(place);
        if !self.0.engine.knows(doc_id) {
            return Err(format!("this node does not know {place}"));
        }
        let version = self
            .0
            .engine
            .save_bytes(doc_id, path, bytes)
            .await
            .map_err(|e| e.to_string())?;
        FileAt::of(version.content.as_deref(), version.size)
            .ok_or_else(|| "the save has no content".to_owned())
    }

    /// The whole of a small file (a session document), at most `max`
    /// bytes, from wherever it is; `None` when no holder has it or it is
    /// larger.
    pub async fn read_small(&self, at: FileAt, max: u64) -> Option<Vec<u8>> {
        if at.size > max {
            return None;
        }
        if let Some(path) = self.0.engine.store.file_for(&at.hash) {
            if let Ok(bytes) = tokio::fs::read(&path).await {
                return Some(bytes);
            }
        }
        let blob = tokio::time::timeout(FIRST_BYTE, self.0.engine.fetcher.relay(at.hash, 0))
            .await
            .ok()?
            .ok()?;
        let mut bytes = Vec::new();
        tokio::time::timeout(
            FIRST_BYTE.saturating_mul(2),
            blob.reader.take(max).read_to_end(&mut bytes),
        )
        .await
        .ok()?
        .ok()?;
        Some(bytes)
    }

    async fn bucket_has(&self, bucket: &S3Store, hash: &Sha256) -> bool {
        let known = self
            .0
            .in_bucket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(hash)
            .is_some_and(|at| at.elapsed() < BUCKET_KNOWN);
        if known {
            return true;
        }
        let has = matches!(
            tokio::time::timeout(FIRST_BYTE, bucket.size_of(hash)).await,
            Ok(Ok(Some(_)))
        );
        if has {
            let mut seen = self
                .0
                .in_bucket
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            seen.retain(|_, at| at.elapsed() < BUCKET_KNOWN);
            seen.insert(*hash, Instant::now());
        }
        has
    }

    /// Answer a request for `at` with `headers` (its `Range`), as
    /// `content_type`: from here, the bucket or a peer (the module docs).
    /// `404` when no holder this node can reach has it now.
    pub async fn respond(&self, at: FileAt, content_type: &str, headers: &HeaderMap) -> Response {
        if let Some(path) = self.0.engine.store.file_for(&at.hash) {
            if let Ok(response) = serve_path(headers, &path, content_type).await {
                return from(From::Here, response);
            }
        }
        if let Some(bucket) = &self.0.bucket {
            if self.bucket_has(bucket, &at.hash).await {
                if let Ok(url) = bucket.signed_get(&at.hash, SIGNED_FOR).await {
                    if let Ok(location) = HeaderValue::from_str(&url) {
                        let mut response = StatusCode::FOUND.into_response();
                        response.headers_mut().insert(header::LOCATION, location);
                        return from(From::Bucket, response);
                    }
                }
            }
        }
        self.relay(at, content_type, headers).await
    }

    async fn relay(&self, at: FileAt, content_type: &str, headers: &HeaderMap) -> Response {
        let Ok(Ok(permit)) =
            tokio::time::timeout(RELAY_WAIT, Arc::clone(&self.0.relays).acquire_owned()).await
        else {
            let mut response = (
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many files are being read through this node. Try again in a moment.",
            )
                .into_response();
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
            return response;
        };
        let fetcher = self.0.engine.fetcher.clone();
        let served = architect::http::ranged::serve_body(
            headers,
            at.size,
            content_type,
            |offset, len| async move {
                // The first window before the answer starts, so a holder
                // that is gone is a status, not a cut-off body.
                let first = window(&fetcher, at.hash, offset, len).await?;
                let state = Relayed {
                    fetcher,
                    hash: at.hash,
                    reader: Some(first),
                    offset,
                    left: len,
                    in_window: len.min(WINDOW),
                    _place: permit,
                };
                Ok::<_, StatusCode>(Body::from_stream(futures::stream::unfold(
                    state,
                    Relayed::next,
                )))
            },
        )
        .await;
        match served {
            Ok(response) => from(From::Peer, response),
            Err(StatusCode::NOT_FOUND) => (
                StatusCode::NOT_FOUND,
                "No holder of this file can be reached now.",
            )
                .into_response(),
            Err(status) => (status, "The file's holder did not send it.").into_response(),
        }
    }
}

/// A file on this disk, ranged.
async fn serve_path(
    headers: &HeaderMap,
    path: &Path,
    content_type: &str,
) -> std::io::Result<Response> {
    architect::http::ranged::serve_file(headers, path, content_type).await
}

fn from(from: From, mut response: Response) -> Response {
    response
        .headers_mut()
        .insert("x-architect-from", HeaderValue::from_static(from.label()));
    response
}

/// One window of `hash` from the holder: at most [`WINDOW`] of the `left`
/// bytes from `offset`.
async fn window(
    fetcher: &architect::blob::Fetcher,
    hash: Sha256,
    offset: u64,
    left: u64,
) -> Result<architect::blob::Reader, StatusCode> {
    tokio::time::timeout(
        FIRST_BYTE,
        fetcher.relay_range(hash, offset, left.min(WINDOW)),
    )
    .await
    .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
    .map(|blob| blob.reader)
    .map_err(|e| match e {
        architect::blob::BlobError::NotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_GATEWAY,
    })
}

/// A relayed range, a window at a time: the holder's stream of the
/// window being sent, what is left, and its place among the relays, given
/// back when the body is dropped (sent, or the listener left).
struct Relayed {
    fetcher: architect::blob::Fetcher,
    hash: Sha256,
    reader: Option<architect::blob::Reader>,
    offset: u64,
    /// Bytes of the range still to send, and of the open window.
    left: u64,
    in_window: u64,
    _place: OwnedSemaphorePermit,
}

impl Relayed {
    /// The next piece of the body; `None` at the range's end.
    async fn next(mut self) -> Option<(std::io::Result<Vec<u8>>, Self)> {
        const PIECE: usize = 64 << 10;
        if self.left == 0 {
            return None;
        }
        if self.in_window == 0 {
            match window(&self.fetcher, self.hash, self.offset, self.left).await {
                Ok(reader) => {
                    self.reader = Some(reader);
                    self.in_window = self.left.min(WINDOW);
                }
                Err(status) => {
                    self.left = 0;
                    return Some((
                        Err(std::io::Error::other(format!("the holder: {status}"))),
                        self,
                    ));
                }
            }
        }
        let want = usize::try_from(self.in_window).map_or(PIECE, |w| w.min(PIECE));
        let mut piece = vec![0_u8; want];
        let read = match self.reader.as_mut() {
            Some(reader) => reader.read(&mut piece).await,
            None => Ok(0),
        };
        match read {
            Ok(0) => {
                self.left = 0;
                Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the holder sent less than the file",
                    )),
                    self,
                ))
            }
            Ok(n) => {
                piece.truncate(n);
                let n = u64::try_from(n).unwrap_or(u64::MAX);
                self.offset = self.offset.saturating_add(n);
                self.left = self.left.saturating_sub(n);
                self.in_window = self.in_window.saturating_sub(n);
                Some((Ok(piece), self))
            }
            Err(e) => {
                self.left = 0;
                Some((Err(e), self))
            }
        }
    }
}
