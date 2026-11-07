//! [`S3Store`]: blobs in any S3-compatible service (Garage, `MinIO`,
//! Cloudflare R2, AWS S3, Ceph RGW), content-addressed as [`LocalFs`] is.
//!
//! - **Keys** are `<prefix>sha256/<hex[0..2]>/<hex>`, so every node that
//!   shares a bucket names the same bytes with the same key, and storing
//!   them twice (from two nodes, or two projects) is a no-op.
//! - **One holder.** The bucket (under its prefix) is one holder of
//!   blobs, with a stable id ([`S3Store::holder_id`], `s3:<uuid>`) kept
//!   in the bucket itself (`<prefix>architect-holder`), so every node
//!   that opens it agrees on it, and a node that moves to another
//!   endpoint for the same bucket keeps it.
//! - **Put, delete and verify are here and only here** ([`S3Store::put_reader`],
//!   [`S3Store::delete`], [`S3Store::verify`]), each reported to the
//!   store's [`HolderEvents`], where replica accounting hooks in.
//! - **A put never stages locally.** The body streams to a temporary key
//!   (multipart, 8 MiB parts) while it is hashed; only bytes that hash to
//!   what was expected are copied to their key, server-side.
//! - **Reads are ranged**, so a [`Fetcher`](super::Fetcher) resumes from
//!   its `.part` against an `S3Store` as against a peer: it is a
//!   [`Remote`] as well as a [`BlobStore`].
//!
//! Credentials come from the caller (a node's config, its environment or
//! a secret file); [`S3Config`]'s `Debug` never shows the secret, and
//! nothing here logs it.

use std::fmt;
use std::io;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::BoxFuture;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path as Key;
use object_store::{
    GetOptions, GetRange, ObjectStore as _, ObjectStoreExt as _, PutMode, PutOptions, PutPayload,
    WriteMultipart,
};
use sha2::Digest as _;
use tokio::io::AsyncReadExt as _;

use super::stream_reader::StreamReader;
use super::{Blob, BlobError, BlobStore, Reader, Remote, Sha256};

/// A multipart upload's part size.
const PART: usize = 8 << 20;

/// The object that names the holder.
const HOLDER: &str = "architect-holder";

/// Where an [`S3Store`] keeps its blobs, and as whom it signs.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct S3Config {
    /// `https://s3.garage.example:3900`; `None` is AWS.
    pub endpoint: Option<String>,
    /// `garage`, `auto` (R2) or an AWS region; default `us-east-1`.
    pub region: String,
    pub bucket: String,
    /// Keys go under it (`architect/`), so a bucket can hold other things.
    pub prefix: String,
    pub access_key_id: String,
    /// Never shown by `Debug`.
    pub secret_access_key: String,
    /// `bucket.host` rather than `host/bucket` (AWS and R2 take both;
    /// Garage and `MinIO` want the path style, the default).
    pub virtual_hosted: bool,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// What a holder of blobs did: where replica accounting (attestations,
/// scrubs) hooks in. Called after the bucket answered.
pub trait HolderEvents: Send + Sync {
    /// `holder` holds `hash` (`size` bytes): stored now, or found there.
    fn held(&self, holder: &str, hash: Sha256, size: u64);
    /// `holder` no longer holds `hash`.
    fn dropped(&self, holder: &str, hash: Sha256);
    /// `holder`'s copy of `hash` was read back whole: `ok` when it hashed
    /// to its name.
    fn verified(&self, holder: &str, hash: Sha256, ok: bool);
}

/// A [`BlobStore`] and [`Remote`] in an S3 bucket. Cheap to clone. See
/// the [module docs](self).
#[derive(Clone)]
pub struct S3Store {
    s3: Arc<AmazonS3>,
    prefix: String,
    bucket: String,
    holder: String,
    events: Option<Arc<dyn HolderEvents>>,
}

impl fmt::Debug for S3Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Store")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("holder", &self.holder)
            .finish_non_exhaustive()
    }
}

fn source(e: object_store::Error) -> BlobError {
    BlobError::Source(io::Error::other(e))
}

impl S3Store {
    /// Open the bucket of `config`, learning its holder id (made on the
    /// first open).
    ///
    /// # Errors
    ///
    /// The configuration is incomplete, or the bucket does not answer.
    pub async fn open(config: &S3Config) -> Result<Self, BlobError> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_region(if config.region.is_empty() {
                "us-east-1"
            } else {
                &config.region
            })
            .with_access_key_id(&config.access_key_id)
            .with_secret_access_key(&config.secret_access_key)
            .with_virtual_hosted_style_request(config.virtual_hosted);
        if let Some(endpoint) = &config.endpoint {
            builder = builder
                .with_endpoint(endpoint)
                .with_allow_http(endpoint.starts_with("http://"));
        }
        let s3 = Arc::new(builder.build().map_err(source)?);
        let prefix = match config.prefix.trim_matches('/') {
            "" => String::new(),
            p => format!("{p}/"),
        };
        let holder = holder_id(&s3, &prefix).await?;
        Ok(Self {
            s3,
            prefix,
            bucket: config.bucket.clone(),
            holder,
            events: None,
        })
    }

    /// Report puts, deletes and verifies to `events`.
    #[must_use]
    pub fn with_events(mut self, events: Arc<dyn HolderEvents>) -> Self {
        self.events = Some(events);
        self
    }

    /// `s3:<uuid>`: this bucket and prefix as a holder of blobs, the same
    /// from every node that opens it.
    #[must_use]
    pub fn holder_id(&self) -> &str {
        &self.holder
    }

    /// The bucket's name.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The key of `hash`: `<prefix>sha256/<hex[0..2]>/<hex>`.
    #[must_use]
    pub fn key_for(&self, hash: &Sha256) -> String {
        let hex = hash.to_hex();
        format!(
            "{}sha256/{}/{hex}",
            self.prefix,
            hex.get(..2).unwrap_or_default()
        )
    }

    fn key(&self, hash: &Sha256) -> Key {
        Key::from(self.key_for(hash))
    }

    /// The blob's size, or `None` when the bucket does not hold it.
    ///
    /// # Errors
    ///
    /// The bucket does not answer.
    pub async fn size_of(&self, hash: &Sha256) -> Result<Option<u64>, BlobError> {
        match self.s3.head(&self.key(hash)).await {
            Ok(meta) => Ok(Some(meta.size)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(source(e)),
        }
    }

    /// Store `body`; see [`BlobStore::put`]. The bytes stream to a
    /// temporary key while they are hashed and are copied to their own
    /// key only when they hash right; bytes the bucket holds already are
    /// not copied again.
    ///
    /// # Errors
    ///
    /// The body fails, the bytes hash to something else, or the bucket
    /// does not answer.
    pub async fn put_reader(
        &self,
        expected: Option<Sha256>,
        mut body: Reader,
    ) -> Result<Sha256, BlobError> {
        if let Some(hash) = expected
            && let Some(size) = self.size_of(&hash).await?
        {
            self.emit(|e, h| e.held(h, hash, size));
            return Ok(hash);
        }
        let tmp = Key::from(format!("{}tmp/{}", self.prefix, uuid::Uuid::new_v4()));
        let mut upload = WriteMultipart::new_with_chunk_size(
            self.s3.put_multipart(&tmp).await.map_err(source)?,
            PART,
        );
        let mut hasher = sha2::Sha256::new();
        let mut size = 0u64;
        let mut buf = vec![0u8; super::BUF];
        let streamed: Result<(), BlobError> = async {
            loop {
                let n = body.read(&mut buf).await.map_err(BlobError::Source)?;
                let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
                    return Ok(());
                };
                upload.wait_for_capacity(4).await.map_err(source)?;
                upload.write(chunk);
                hasher.update(chunk);
                size = size.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
            }
        }
        .await;
        if let Err(e) = streamed {
            let _ = upload.abort().await;
            return Err(e);
        }
        upload.finish().await.map_err(source)?;
        let actual = Sha256(hasher.finalize().into());
        let placed = self.place(&tmp, expected, actual).await;
        let _ = self.s3.delete(&tmp).await;
        placed?;
        self.emit(|e, h| e.held(h, actual, size));
        Ok(actual)
    }

    /// The staged `tmp` to the key of `actual`, if it is what was expected.
    async fn place(
        &self,
        tmp: &Key,
        expected: Option<Sha256>,
        actual: Sha256,
    ) -> Result<(), BlobError> {
        if let Some(expected) = expected.filter(|want| *want != actual) {
            return Err(BlobError::HashMismatch {
                path: std::path::PathBuf::from(tmp.as_ref()),
                expected,
                actual,
            });
        }
        if self.size_of(&actual).await?.is_none() {
            self.s3
                .copy(tmp, &self.key(&actual))
                .await
                .map_err(source)?;
        }
        Ok(())
    }

    /// Remove `hash` from the bucket.
    ///
    /// # Errors
    ///
    /// The bucket does not answer.
    pub async fn delete(&self, hash: Sha256) -> Result<(), BlobError> {
        match self.s3.delete(&self.key(&hash)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => {}
            Err(e) => return Err(source(e)),
        }
        self.emit(|e, h| e.dropped(h, hash));
        Ok(())
    }

    /// Read `hash` back whole and check it hashes to its name: a scrub.
    /// `Ok(false)` when it does not (or is gone).
    ///
    /// # Errors
    ///
    /// The bucket does not answer.
    pub async fn verify(&self, hash: Sha256) -> Result<bool, BlobError> {
        let ok = match self.s3.get(&self.key(&hash)).await {
            Ok(got) => {
                let mut stream = got.into_stream();
                let mut hasher = sha2::Sha256::new();
                while let Some(chunk) = stream.next().await {
                    hasher.update(chunk.map_err(source)?);
                }
                Sha256(hasher.finalize().into()) == hash
            }
            Err(object_store::Error::NotFound { .. }) => false,
            Err(e) => return Err(source(e)),
        };
        self.emit(|e, h| e.verified(h, hash, ok));
        Ok(ok)
    }

    /// Every blob the bucket holds, with its size.
    ///
    /// # Errors
    ///
    /// The bucket does not answer.
    pub async fn list(&self) -> Result<Vec<(Sha256, u64)>, BlobError> {
        let under = Key::from(format!("{}sha256", self.prefix));
        let mut out = Vec::new();
        let mut listing = self.s3.list(Some(&under));
        while let Some(meta) = listing.next().await {
            let meta = meta.map_err(source)?;
            if let Some(hash) = meta
                .location
                .filename()
                .and_then(|name| Sha256::from_hex(name).ok())
            {
                out.push((hash, meta.size));
            }
        }
        Ok(out)
    }

    /// The blob from `offset` (clamped to its end).
    async fn read_at(&self, hash: Sha256, offset: u64) -> Result<Blob, BlobError> {
        let size = self
            .size_of(&hash)
            .await?
            .ok_or(BlobError::NotFound(hash))?;
        let offset = offset.min(size);
        if offset == size {
            return Ok(Blob {
                len: 0,
                reader: Box::pin(tokio::io::empty()),
            });
        }
        let options = GetOptions {
            range: Some(GetRange::Offset(offset)),
            ..GetOptions::default()
        };
        let got = self
            .s3
            .get_opts(&self.key(&hash), options)
            .await
            .map_err(source)?;
        let stream = got
            .into_stream()
            .map(|chunk| chunk.map(|b| b.to_vec()).map_err(io::Error::other));
        Ok(Blob {
            len: size.saturating_sub(offset),
            reader: Box::pin(StreamReader::new(stream)),
        })
    }

    fn emit(&self, f: impl FnOnce(&dyn HolderEvents, &str)) {
        if let Some(events) = &self.events {
            f(events.as_ref(), &self.holder);
        }
    }
}

/// The holder id kept in the bucket, made by the first node to open it.
async fn holder_id(s3: &AmazonS3, prefix: &str) -> Result<String, BlobError> {
    let at = Key::from(format!("{prefix}{HOLDER}"));
    if let Some(id) = read_holder(s3, &at).await? {
        return Ok(id);
    }
    let mine = format!("s3:{}", uuid::Uuid::new_v4());
    let create = PutOptions {
        mode: PutMode::Create,
        ..PutOptions::default()
    };
    match s3
        .put_opts(&at, PutPayload::from(mine.clone().into_bytes()), create)
        .await
    {
        // Another node made it first: theirs.
        Ok(_) | Err(object_store::Error::AlreadyExists { .. }) => {}
        // No conditional writes here: a plain write, then whatever stuck.
        Err(
            object_store::Error::NotImplemented { .. } | object_store::Error::Precondition { .. },
        ) => {
            s3.put(&at, PutPayload::from(mine.clone().into_bytes()))
                .await
                .map_err(source)?;
        }
        Err(e) => return Err(source(e)),
    }
    Ok(read_holder(s3, &at).await?.unwrap_or(mine))
}

async fn read_holder(s3: &AmazonS3, at: &Key) -> Result<Option<String>, BlobError> {
    match s3.get(at).await {
        Ok(got) => {
            let bytes = got.bytes().await.map_err(source)?;
            let id = String::from_utf8_lossy(&bytes).trim().to_owned();
            Ok(id.starts_with("s3:").then_some(id))
        }
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(source(e)),
    }
}

impl BlobStore for S3Store {
    fn put(
        &self,
        expected: Option<Sha256>,
        body: Reader,
    ) -> BoxFuture<'_, Result<Sha256, BlobError>> {
        Box::pin(self.put_reader(expected, body))
    }

    fn get(&self, hash: Sha256) -> BoxFuture<'_, Result<Blob, BlobError>> {
        Box::pin(self.read_at(hash, 0))
    }

    fn has(&self, hash: Sha256) -> BoxFuture<'_, Result<bool, BlobError>> {
        Box::pin(async move { Ok(self.size_of(&hash).await?.is_some()) })
    }

    fn get_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, Result<Blob, BlobError>> {
        Box::pin(self.read_at(hash, offset))
    }
}

impl Remote for S3Store {
    fn size(&self, hash: Sha256) -> BoxFuture<'_, io::Result<Option<u64>>> {
        Box::pin(async move { self.size_of(&hash).await.map_err(io::Error::other) })
    }

    fn read_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        Box::pin(async move {
            self.read_at(hash, offset)
                .await
                .map(|b| b.reader)
                .map_err(io::Error::other)
        })
    }
}
