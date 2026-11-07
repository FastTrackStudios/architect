//! A device's roots over the S3 API: the same roots, versions and
//! permissions as the `WebDAV` face, for `rclone`, `aws s3`, `restic` and
//! anything else that speaks S3 (feature `s3`).
//!
//! - **Buckets and keys** ([`map`]): a bucket per org, keys
//!   `Projects/<project>/…` and `Assets/<kind>/…`.
//! - **Versions are the manifest's.** A `PutObject` or a completed
//!   multipart upload saves a version; its id is the S3 version id.
//!   `ListObjectVersions` lists a path's history, a delete is a delete
//!   marker, and `GetObject?versionId=` reads a past version. Versioning
//!   is always on and cannot be suspended; history is append-only, so a
//!   delete of one version is refused.
//! - **The `ETag` is the content's sha-256** (64 hex digits), the same
//!   id the manifest and the blob store use; a part's is its MD5, as a
//!   client checks it.
//! - **`SigV4`** is `s3s`'s, against [`S3Keys`]; what a key may do is its
//!   role, lowered to its holder's now ([`KeyRoles`]), through
//!   `access::resolve`, as `FilesServer`'s `Acting` checks it. A path the
//!   key may not see is absent (`NoSuchKey`), as everywhere in files.
//! - **Bytes** come from this device's store or working copy, else are
//!   streamed from a peer as they arrive (`Fetcher::open_at`), as a
//!   `WebDAV` `GET`.
//!
//! What it speaks: `ListBuckets`, `HeadBucket`, `GetBucketLocation`,
//! `GetBucketVersioning`, `ListObjects` (v1 and v2, prefix, delimiter,
//! pages), `ListObjectVersions`, `HeadObject`, `GetObject` (with `Range`
//! and `versionId`), `PutObject`, `CopyObject`, `DeleteObject`,
//! `DeleteObjects`, and multipart (`Create`, `UploadPart`, `Complete`,
//! `Abort`). Buckets are orgs, so `CreateBucket` is not.

pub mod keys;
pub mod map;
mod uploads;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use architect::blob::Sha256;
use bytes::Bytes;
use futures_util::StreamExt as _;
use s3s::dto::{self, BucketVersioningStatus, CommonPrefix, ETag, StreamingBlob, Timestamp};
use s3s::{S3, S3Request, S3Response, S3Result, s3_error};
use tokio::io::AsyncReadExt as _;
use uuid::Uuid;

pub use keys::{AsIssued, KeyHolder, KeyRoles, S3Key, S3Keys};

use super::engine::Engine;
use super::manifest::{FileVersion, Tree, doc_id_for};
use crate::access::{Access, Capability, resolve, role_capabilities};
use crate::id::VersionId;
use crate::path::RootPath;
use crate::roots::RootMeta;
use uploads::Uploads;

/// How many keys a listing returns when the client does not say.
const MAX_KEYS: usize = 1000;

/// The S3 face over an [`Engine`]. Cheap to clone.
#[derive(Clone)]
pub struct S3Face {
    engine: Engine,
    keys: S3Keys,
    roles: Arc<dyn KeyRoles>,
    uploads: Uploads,
}

/// The caller of one request: its key and what it may do now.
struct Caller {
    key: S3Key,
    caps: Vec<Capability>,
}

/// Where a key lands: a root this device knows, and the path in it.
struct At {
    doc_id: Uuid,
    meta: RootMeta,
    path: String,
}

impl S3Face {
    /// The face over `engine`, its keys in `keys`, each key's role now
    /// from `roles`. In-progress multipart uploads are staged under the
    /// engine's data dir.
    #[must_use]
    pub fn new(engine: Engine, keys: S3Keys, roles: impl KeyRoles) -> Self {
        let uploads = Uploads::new(engine.config().data.join("s3-uploads"));
        Self {
            engine,
            keys,
            roles: Arc::new(roles),
            uploads,
        }
    }

    /// The `s3s` service: `SigV4` against the keys, path-style buckets.
    #[must_use]
    pub fn service(&self) -> s3s::service::S3Service {
        let mut builder = s3s::service::S3ServiceBuilder::new(self.clone());
        builder.set_auth(Auth(self.keys.clone()));
        builder.set_validation(OrgNames);
        builder.build()
    }

    /// Serve [`Self::service`] on `listener` until the task is dropped.
    pub async fn serve(self, listener: tokio::net::TcpListener) {
        let service = self.service();
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let service = service.clone();
            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let builder = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                );
                if let Err(error) = builder.serve_connection(io, service).await {
                    tracing::debug!(%error, "s3: connection closed");
                }
            });
        }
    }

    fn caller<T>(&self, req: &S3Request<T>) -> S3Result<Caller> {
        let creds = req
            .credentials
            .as_ref()
            .ok_or_else(|| s3_error!(AccessDenied, "sign the request with an access key"))?;
        let key = self
            .keys
            .get(&creds.access_key)
            .ok_or_else(|| s3_error!(InvalidAccessKeyId))?;
        let role = self
            .roles
            .role_now(&key)
            .map(|now| keys::lesser(key.role.clone(), now))
            .ok_or_else(|| s3_error!(AccessDenied, "the key's holder holds nothing here now"))?;
        Ok(Caller {
            caps: role_capabilities(&role).to_vec(),
            key,
        })
    }

    /// The bucket is the caller's org, and this device holds some of it.
    async fn bucket(&self, caller: &Caller, bucket: &str) -> S3Result<()> {
        let held = self.engine.status().await.iter().any(|r| r.org == bucket);
        if caller.key.org == bucket && held {
            Ok(())
        } else {
            Err(s3_error!(NoSuchBucket))
        }
    }

    /// What the caller may do at `path` in a root: `access::resolve`
    /// over the role's capabilities (no per-folder grants on synced
    /// roots yet).
    fn access(caller: &Caller, meta: &RootMeta, path: &str) -> Access {
        let at = RootPath::parse(path).unwrap_or_else(|_| RootPath::root());
        resolve(
            &caller.caps,
            &[],
            Some(&caller.key.holder.label()),
            meta.id,
            &at,
            super::engine::now_us(),
        )
    }

    /// Refuse unless the caller may do `cap` at `at`: absent when they
    /// hold nothing there, denied when they see it and may not do this.
    fn need(caller: &Caller, at: &At, cap: Capability) -> S3Result<()> {
        match Self::access(caller, &at.meta, &at.path) {
            Access::Absent => Err(s3_error!(NoSuchKey)),
            a if a.allows(cap) => Ok(()),
            Access::Holds(_) => Err(s3_error!(AccessDenied)),
        }
    }

    /// The root and path `key` names in `bucket`.
    fn locate(&self, bucket: &str, key: &str) -> S3Result<At> {
        let (section, name, path) = map::split_key(key).ok_or_else(|| s3_error!(NoSuchKey))?;
        let place = format!("{bucket}/{}/{name}", section.dir());
        let doc_id = doc_id_for(&place);
        let meta = self
            .engine
            .meta(doc_id)
            .filter(|_| self.engine.knows(doc_id))
            .ok_or_else(|| s3_error!(NoSuchKey))?;
        if RootPath::parse(path).is_err() {
            return Err(s3_error!(InvalidArgument, "not a path in a root"));
        }
        Ok(At {
            doc_id,
            meta,
            path: path.to_owned(),
        })
    }

    async fn tree(&self, at: &At) -> S3Result<Tree> {
        self.engine
            .tree(at.doc_id)
            .await
            .map_err(|_| s3_error!(NoSuchKey))
    }

    /// The version a read names: `version_id`, else the current one.
    async fn version(
        &self,
        caller: &Caller,
        at: &At,
        version_id: Option<&str>,
    ) -> S3Result<FileVersion> {
        Self::need(caller, at, Capability::Read)?;
        let tree = self.tree(at).await?;
        match version_id.filter(|v| *v != "null") {
            Some(id) => {
                Self::need(caller, at, Capability::History)?;
                let id = Uuid::parse_str(id).map_err(|_| s3_error!(NoSuchVersion))?;
                let v = tree
                    .version(VersionId(id))
                    .filter(|v| v.path == at.path)
                    .cloned()
                    .ok_or_else(|| s3_error!(NoSuchVersion))?;
                if v.content.is_none() {
                    return Err(s3_error!(MethodNotAllowed, "a delete marker"));
                }
                Ok(v)
            }
            None => tree.current(&at.path).ok_or_else(|| s3_error!(NoSuchKey)),
        }
    }

    /// The bytes of `content` from `offset`: this device's, else a peer's
    /// as they arrive.
    async fn open(&self, content: &Sha256, offset: u64) -> S3Result<architect::blob::Reader> {
        let opened = match self.engine.store.file_for(content) {
            Some(file) => super::store::open_from(&file, offset)
                .await
                .map(|b| b.reader)
                .map_err(|e| e.to_string()),
            None => self
                .engine
                .fetcher
                .open_at(*content, offset)
                .await
                .map(|b| b.reader)
                .map_err(|e| e.to_string()),
        };
        opened.map_err(|e| {
            s3_error!(
                ServiceUnavailable,
                "the bytes are on no reachable peer: {e}"
            )
        })
    }

    /// Save `body` at `at` as a new version, then bring the working copy
    /// to it in the background.
    async fn save(&self, at: &At, body: architect::blob::Reader) -> S3Result<FileVersion> {
        if super::layout::ignored(&at.path) {
            return Err(s3_error!(InvalidArgument, "a name files never syncs"));
        }
        let saved = self
            .engine
            .save_reader(at.doc_id, &at.path, body)
            .await
            .map_err(|e| s3_error!(InternalError, "{e}"))?;
        self.pass_soon(at.doc_id);
        Ok(saved)
    }

    fn pass_soon(&self, doc_id: Uuid) {
        let engine = self.engine.clone();
        tokio::spawn(async move { engine.pass(doc_id).await });
    }

    /// Every live key of `bucket` under `prefix` the caller may read.
    async fn keys_under(
        &self,
        caller: &Caller,
        bucket: &str,
        prefix: &str,
    ) -> BTreeMap<String, FileVersion> {
        let mut out = BTreeMap::new();
        for root in self.engine.status().await {
            let Ok(doc_id) = Uuid::parse_str(&root.doc_id) else {
                continue;
            };
            let Some(meta) = self.engine.meta(doc_id) else {
                continue;
            };
            if root.org != bucket || !map::root_may_match(&meta, prefix) {
                continue;
            }
            let Ok(tree) = self.engine.tree(doc_id).await else {
                continue;
            };
            for (path, v) in tree.live() {
                if Self::access(caller, &meta, &path).allows(Capability::Read) {
                    out.insert(map::key_of(&meta, &path), v);
                }
            }
        }
        out
    }

    /// Every version (newest first per key) of `bucket` under `prefix`.
    async fn versions_under(
        &self,
        caller: &Caller,
        bucket: &str,
        prefix: &str,
    ) -> BTreeMap<String, Vec<FileVersion>> {
        let mut out = BTreeMap::new();
        for root in self.engine.status().await {
            let Ok(doc_id) = Uuid::parse_str(&root.doc_id) else {
                continue;
            };
            let Some(meta) = self.engine.meta(doc_id) else {
                continue;
            };
            if root.org != bucket || !map::root_may_match(&meta, prefix) {
                continue;
            }
            let Ok(tree) = self.engine.tree(doc_id).await else {
                continue;
            };
            for path in tree.paths() {
                let access = Self::access(caller, &meta, path);
                if !(access.allows(Capability::Read) && access.allows(Capability::History)) {
                    continue;
                }
                let mut list = tree.history(path).to_vec();
                list.sort_by(|a, b| b.at_us.cmp(&a.at_us).then(b.id.0.cmp(&a.id.0)));
                out.insert(map::key_of(&meta, path), list);
            }
        }
        out
    }

    async fn delete_one(
        &self,
        caller: &Caller,
        bucket: &str,
        key: &str,
        version: Option<&str>,
    ) -> S3Result<bool> {
        if version.is_some_and(|v| v != "null") {
            return Err(s3_error!(
                NotImplemented,
                "history is append-only: delete the key, not one of its versions"
            ));
        }
        let Ok(at) = self.locate(bucket, key) else {
            return Ok(false);
        };
        Self::need(caller, &at, Capability::Write)?;
        let tree = self.tree(&at).await?;
        if tree.current(&at.path).is_none() {
            return Ok(false);
        }
        self.engine
            .delete_path(at.doc_id, &at.path)
            .await
            .map_err(|e| s3_error!(InternalError, "{e}"))?;
        self.pass_soon(at.doc_id);
        Ok(true)
    }
}

/// `SigV4` secrets from the keys.
struct Auth(S3Keys);

#[async_trait::async_trait]
impl s3s::auth::S3Auth for Auth {
    async fn get_secret_key(&self, access_key: &str) -> S3Result<s3s::auth::SecretKey> {
        self.0
            .get(access_key)
            .map(|k| s3s::auth::SecretKey::from(k.secret))
            .ok_or_else(|| s3_error!(InvalidAccessKeyId))
    }
}

/// A bucket is an org's folder name, whatever its case.
struct OrgNames;

impl s3s::validation::NameValidation for OrgNames {
    fn validate_bucket_name(&self, name: &str) -> bool {
        !name.is_empty() && !name.contains('/') && name != "." && name != ".."
    }
}

fn etag(content: Option<&str>) -> Option<ETag> {
    Some(ETag::Strong(content?.to_owned()))
}

fn when(at_us: i64) -> Timestamp {
    let at = UNIX_EPOCH
        .checked_add(Duration::from_micros(u64::try_from(at_us).unwrap_or(0)))
        .unwrap_or(UNIX_EPOCH);
    Timestamp::from(at)
}

fn size(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn max_keys(asked: Option<i32>) -> usize {
    asked
        .and_then(|n| usize::try_from(n).ok())
        .map_or(MAX_KEYS, |n| n.min(MAX_KEYS))
}

fn object(key: String, v: &FileVersion) -> dto::Object {
    dto::Object {
        key: Some(key),
        size: Some(size(v.size)),
        e_tag: etag(v.content.as_deref()),
        last_modified: Some(when(v.at_us)),
        ..Default::default()
    }
}

fn prefixes(list: Vec<String>) -> Vec<CommonPrefix> {
    list.into_iter()
        .map(|p| CommonPrefix { prefix: Some(p) })
        .collect()
}

/// A reader as a response body: read on its own task, so the body is
/// `Sync` as `s3s` wants it.
fn body_of(mut reader: architect::blob::Reader) -> StreamingBlob {
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(4);
    tokio::spawn(async move {
        let mut buf = vec![0u8; 256 << 10];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = Bytes::copy_from_slice(buf.get(..n).unwrap_or_default());
                    if tx.send(Ok(chunk)).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            }
        }
    });
    StreamingBlob::wrap(Chunks(rx))
}

struct Chunks(tokio::sync::mpsc::Receiver<std::io::Result<Bytes>>);

impl futures_util::Stream for Chunks {
    type Item = std::io::Result<Bytes>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

/// A request body as a reader.
fn reader_of(body: Option<StreamingBlob>) -> architect::blob::Reader {
    match body {
        Some(blob) => Box::pin(tokio_util::io::StreamReader::new(
            blob.map(|chunk| chunk.map_err(std::io::Error::other)),
        )),
        None => Box::pin(std::io::Cursor::new(Vec::new())),
    }
}

#[async_trait::async_trait]
impl S3 for S3Face {
    async fn list_buckets(
        &self,
        req: S3Request<dto::ListBucketsInput>,
    ) -> S3Result<S3Response<dto::ListBucketsOutput>> {
        let caller = self.caller(&req)?;
        let org = caller.key.org.clone();
        let held = self.engine.status().await.iter().any(|r| r.org == org);
        let buckets = held.then(|| dto::Bucket {
            name: Some(org),
            creation_date: Some(Timestamp::from(SystemTime::now())),
            ..Default::default()
        });
        Ok(S3Response::new(dto::ListBucketsOutput {
            buckets: Some(buckets.into_iter().collect()),
            ..Default::default()
        }))
    }

    async fn head_bucket(
        &self,
        req: S3Request<dto::HeadBucketInput>,
    ) -> S3Result<S3Response<dto::HeadBucketOutput>> {
        let caller = self.caller(&req)?;
        self.bucket(&caller, &req.input.bucket).await?;
        Ok(S3Response::new(dto::HeadBucketOutput::default()))
    }

    async fn create_bucket(
        &self,
        req: S3Request<dto::CreateBucketInput>,
    ) -> S3Result<S3Response<dto::CreateBucketOutput>> {
        let caller = self.caller(&req)?;
        if self.bucket(&caller, &req.input.bucket).await.is_ok() {
            return Err(s3_error!(BucketAlreadyOwnedByYou));
        }
        Err(s3_error!(
            AccessDenied,
            "a bucket is an org: made at the issuer, not here"
        ))
    }

    async fn get_bucket_location(
        &self,
        req: S3Request<dto::GetBucketLocationInput>,
    ) -> S3Result<S3Response<dto::GetBucketLocationOutput>> {
        let caller = self.caller(&req)?;
        self.bucket(&caller, &req.input.bucket).await?;
        Ok(S3Response::new(dto::GetBucketLocationOutput::default()))
    }

    async fn get_bucket_versioning(
        &self,
        req: S3Request<dto::GetBucketVersioningInput>,
    ) -> S3Result<S3Response<dto::GetBucketVersioningOutput>> {
        let caller = self.caller(&req)?;
        self.bucket(&caller, &req.input.bucket).await?;
        Ok(S3Response::new(dto::GetBucketVersioningOutput {
            status: Some(BucketVersioningStatus::from_static(
                BucketVersioningStatus::ENABLED,
            )),
            ..Default::default()
        }))
    }

    async fn list_objects_v2(
        &self,
        req: S3Request<dto::ListObjectsV2Input>,
    ) -> S3Result<S3Response<dto::ListObjectsV2Output>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let prefix = input.prefix.clone().unwrap_or_default();
        let after = input
            .continuation_token
            .clone()
            .or_else(|| input.start_after.clone());
        let keys = self.keys_under(&caller, &input.bucket, &prefix).await;
        let page = map::page(
            &keys,
            &prefix,
            input.delimiter.as_deref(),
            after.as_deref(),
            max_keys(input.max_keys),
        );
        let count = page.objects.len().saturating_add(page.prefixes.len());
        Ok(S3Response::new(dto::ListObjectsV2Output {
            name: Some(input.bucket),
            prefix: input.prefix,
            delimiter: input.delimiter,
            max_keys: input.max_keys.or(Some(1000)),
            key_count: Some(i32::try_from(count).unwrap_or(i32::MAX)),
            continuation_token: input.continuation_token,
            start_after: input.start_after,
            is_truncated: Some(page.next.is_some()),
            next_continuation_token: page.next,
            contents: Some(
                page.objects
                    .into_iter()
                    .map(|(k, v)| object(k, &v))
                    .collect(),
            ),
            common_prefixes: Some(prefixes(page.prefixes)),
            encoding_type: input.encoding_type,
            ..Default::default()
        }))
    }

    async fn list_objects(
        &self,
        req: S3Request<dto::ListObjectsInput>,
    ) -> S3Result<S3Response<dto::ListObjectsOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let prefix = input.prefix.clone().unwrap_or_default();
        let keys = self.keys_under(&caller, &input.bucket, &prefix).await;
        let page = map::page(
            &keys,
            &prefix,
            input.delimiter.as_deref(),
            input.marker.as_deref(),
            max_keys(input.max_keys),
        );
        Ok(S3Response::new(dto::ListObjectsOutput {
            name: Some(input.bucket),
            prefix: input.prefix,
            delimiter: input.delimiter,
            marker: input.marker,
            max_keys: input.max_keys.or(Some(1000)),
            is_truncated: Some(page.next.is_some()),
            next_marker: page.next,
            contents: Some(
                page.objects
                    .into_iter()
                    .map(|(k, v)| object(k, &v))
                    .collect(),
            ),
            common_prefixes: Some(prefixes(page.prefixes)),
            encoding_type: input.encoding_type,
            ..Default::default()
        }))
    }

    async fn list_object_versions(
        &self,
        req: S3Request<dto::ListObjectVersionsInput>,
    ) -> S3Result<S3Response<dto::ListObjectVersionsOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let prefix = input.prefix.clone().unwrap_or_default();
        let all = self.versions_under(&caller, &input.bucket, &prefix).await;
        let page = map::page(
            &all,
            &prefix,
            input.delimiter.as_deref(),
            input.key_marker.as_deref(),
            max_keys(input.max_keys),
        );
        let (mut versions, mut markers) = (Vec::new(), Vec::new());
        for (key, list) in page.objects {
            for (i, v) in list.iter().enumerate() {
                let (latest, id) = (Some(i == 0), Some(v.id.0.to_string()));
                if v.content.is_some() {
                    versions.push(dto::ObjectVersion {
                        key: Some(key.clone()),
                        version_id: id,
                        is_latest: latest,
                        size: Some(size(v.size)),
                        e_tag: etag(v.content.as_deref()),
                        last_modified: Some(when(v.at_us)),
                        ..Default::default()
                    });
                } else {
                    markers.push(dto::DeleteMarkerEntry {
                        key: Some(key.clone()),
                        version_id: id,
                        is_latest: latest,
                        last_modified: Some(when(v.at_us)),
                        ..Default::default()
                    });
                }
            }
        }
        Ok(S3Response::new(dto::ListObjectVersionsOutput {
            name: Some(input.bucket),
            prefix: input.prefix,
            delimiter: input.delimiter,
            key_marker: input.key_marker,
            max_keys: input.max_keys.or(Some(1000)),
            is_truncated: Some(page.next.is_some()),
            next_key_marker: page.next,
            versions: Some(versions),
            delete_markers: Some(markers),
            common_prefixes: Some(prefixes(page.prefixes)),
            encoding_type: input.encoding_type,
            ..Default::default()
        }))
    }

    async fn head_object(
        &self,
        req: S3Request<dto::HeadObjectInput>,
    ) -> S3Result<S3Response<dto::HeadObjectOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let at = self.locate(&input.bucket, &input.key)?;
        let v = self
            .version(&caller, &at, input.version_id.as_deref())
            .await?;
        Ok(S3Response::new(dto::HeadObjectOutput {
            content_length: Some(size(v.size)),
            content_type: Some(super::dav::mime_of(&at.path).to_owned()),
            e_tag: etag(v.content.as_deref()),
            last_modified: Some(when(v.at_us)),
            version_id: Some(v.id.0.to_string()),
            accept_ranges: Some("bytes".to_owned()),
            ..Default::default()
        }))
    }

    async fn get_object(
        &self,
        req: S3Request<dto::GetObjectInput>,
    ) -> S3Result<S3Response<dto::GetObjectOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let at = self.locate(&input.bucket, &input.key)?;
        let v = self
            .version(&caller, &at, input.version_id.as_deref())
            .await?;
        let content = v
            .content
            .as_deref()
            .and_then(|c| Sha256::from_hex(c).ok())
            .ok_or_else(|| s3_error!(NoSuchKey))?;
        let span = match &input.range {
            Some(range) => Some(range.check(v.size)?),
            None => None,
        };
        let (start, len) = span
            .as_ref()
            .map_or((0, v.size), |r| (r.start, r.end.saturating_sub(r.start)));
        let reader = self.open(&content, start).await?;
        Ok(S3Response::new(dto::GetObjectOutput {
            body: Some(body_of(Box::pin(reader.take(len)))),
            content_length: Some(size(len)),
            content_range: span
                .map(|r| format!("bytes {}-{}/{}", r.start, r.end.saturating_sub(1), v.size)),
            content_type: Some(super::dav::mime_of(&at.path).to_owned()),
            e_tag: etag(v.content.as_deref()),
            last_modified: Some(when(v.at_us)),
            version_id: Some(v.id.0.to_string()),
            accept_ranges: Some("bytes".to_owned()),
            ..Default::default()
        }))
    }

    async fn put_object(
        &self,
        req: S3Request<dto::PutObjectInput>,
    ) -> S3Result<S3Response<dto::PutObjectOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        // A folder marker (`dir/`): the manifest holds files only.
        if input.key.ends_with('/') {
            return Ok(S3Response::new(dto::PutObjectOutput::default()));
        }
        let at = self.locate(&input.bucket, &input.key)?;
        Self::need(&caller, &at, Capability::Write)?;
        let v = self.save(&at, reader_of(input.body)).await?;
        Ok(S3Response::new(dto::PutObjectOutput {
            e_tag: etag(v.content.as_deref()),
            version_id: Some(v.id.0.to_string()),
            size: Some(size(v.size)),
            ..Default::default()
        }))
    }

    async fn copy_object(
        &self,
        req: S3Request<dto::CopyObjectInput>,
    ) -> S3Result<S3Response<dto::CopyObjectOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let dto::CopySource::Bucket {
            bucket,
            key,
            version_id,
        } = &input.copy_source
        else {
            return Err(s3_error!(NotImplemented, "copy from an access point"));
        };
        if **bucket != *input.bucket {
            return Err(s3_error!(AccessDenied, "copy within one org's bucket"));
        }
        let from = self.locate(bucket, key)?;
        let source = self.version(&caller, &from, version_id.as_deref()).await?;
        let to = self.locate(&input.bucket, &input.key)?;
        Self::need(&caller, &to, Capability::Write)?;
        let content = source
            .content
            .as_deref()
            .and_then(|c| Sha256::from_hex(c).ok())
            .ok_or_else(|| s3_error!(NoSuchKey))?;
        let v = self
            .engine
            .save_content(to.doc_id, &to.path, content, source.size)
            .await
            .map_err(|e| s3_error!(InternalError, "{e}"))?;
        self.pass_soon(to.doc_id);
        Ok(S3Response::new(dto::CopyObjectOutput {
            copy_object_result: Some(dto::CopyObjectResult {
                e_tag: etag(v.content.as_deref()),
                last_modified: Some(when(v.at_us)),
                ..Default::default()
            }),
            copy_source_version_id: Some(source.id.0.to_string()),
            version_id: Some(v.id.0.to_string()),
            ..Default::default()
        }))
    }

    async fn delete_object(
        &self,
        req: S3Request<dto::DeleteObjectInput>,
    ) -> S3Result<S3Response<dto::DeleteObjectOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let deleted = self
            .delete_one(
                &caller,
                &input.bucket,
                &input.key,
                input.version_id.as_deref(),
            )
            .await?;
        Ok(S3Response::new(dto::DeleteObjectOutput {
            delete_marker: Some(deleted),
            ..Default::default()
        }))
    }

    async fn delete_objects(
        &self,
        req: S3Request<dto::DeleteObjectsInput>,
    ) -> S3Result<S3Response<dto::DeleteObjectsOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let (mut deleted, mut errors) = (Vec::new(), Vec::new());
        for o in input.delete.objects {
            match self
                .delete_one(&caller, &input.bucket, &o.key, o.version_id.as_deref())
                .await
            {
                Ok(marker) => deleted.push(dto::DeletedObject {
                    key: Some(o.key),
                    delete_marker: Some(marker),
                    ..Default::default()
                }),
                Err(e) => errors.push(dto::Error {
                    code: Some(e.code().as_str().to_owned()),
                    key: Some(o.key),
                    message: e.message().map(str::to_owned),
                    version_id: o.version_id,
                }),
            }
        }
        Ok(S3Response::new(dto::DeleteObjectsOutput {
            deleted: Some(deleted),
            errors: Some(errors),
            ..Default::default()
        }))
    }

    async fn create_multipart_upload(
        &self,
        req: S3Request<dto::CreateMultipartUploadInput>,
    ) -> S3Result<S3Response<dto::CreateMultipartUploadOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        let at = self.locate(&input.bucket, &input.key)?;
        Self::need(&caller, &at, Capability::Write)?;
        let id = self
            .uploads
            .create(&input.bucket, &input.key, &caller.key.access_key)
            .await
            .map_err(|e| s3_error!(InternalError, "{e}"))?;
        Ok(S3Response::new(dto::CreateMultipartUploadOutput {
            bucket: Some(input.bucket),
            key: Some(input.key),
            upload_id: Some(id),
            ..Default::default()
        }))
    }

    async fn upload_part(
        &self,
        req: S3Request<dto::UploadPartInput>,
    ) -> S3Result<S3Response<dto::UploadPartOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.uploads
            .check(
                &input.upload_id,
                &input.bucket,
                &input.key,
                &caller.key.access_key,
            )
            .await?;
        let md5 = self
            .uploads
            .put_part(&input.upload_id, input.part_number, reader_of(input.body))
            .await?;
        Ok(S3Response::new(dto::UploadPartOutput {
            e_tag: Some(ETag::Strong(md5)),
            ..Default::default()
        }))
    }

    async fn complete_multipart_upload(
        &self,
        req: S3Request<dto::CompleteMultipartUploadInput>,
    ) -> S3Result<S3Response<dto::CompleteMultipartUploadOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.bucket(&caller, &input.bucket).await?;
        self.uploads
            .check(
                &input.upload_id,
                &input.bucket,
                &input.key,
                &caller.key.access_key,
            )
            .await?;
        let at = self.locate(&input.bucket, &input.key)?;
        Self::need(&caller, &at, Capability::Write)?;
        let parts: Vec<i32> = input
            .multipart_upload
            .and_then(|m| m.parts)
            .unwrap_or_default()
            .iter()
            .filter_map(|p| p.part_number)
            .collect();
        let body = self.uploads.joined(&input.upload_id, &parts).await?;
        let v = self.save(&at, body).await?;
        self.uploads.remove(&input.upload_id).await;
        Ok(S3Response::new(dto::CompleteMultipartUploadOutput {
            bucket: Some(input.bucket),
            key: Some(input.key),
            e_tag: etag(v.content.as_deref()),
            version_id: Some(v.id.0.to_string()),
            ..Default::default()
        }))
    }

    async fn abort_multipart_upload(
        &self,
        req: S3Request<dto::AbortMultipartUploadInput>,
    ) -> S3Result<S3Response<dto::AbortMultipartUploadOutput>> {
        let caller = self.caller(&req)?;
        let input = req.input;
        self.uploads
            .check(
                &input.upload_id,
                &input.bucket,
                &input.key,
                &caller.key.access_key,
            )
            .await?;
        self.uploads.remove(&input.upload_id).await;
        Ok(S3Response::new(dto::AbortMultipartUploadOutput::default()))
    }
}
