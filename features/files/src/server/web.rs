//! [`WebFiles`] (feature `http`): the [`FilesWeb`] byte face of a
//! [`FilesServer`].
//!
//! The HTTP face is not behind the vox gate, so this resolves the caller
//! itself, the way the gate would: a session token through the gate's
//! own [`IdentityResolver`] (pass the same one — a `MembersResolver` —
//! so a session's orgs are learnt here too), or a read ticket this
//! server signed. A session must be a member of the org in the URL, as
//! the gate's `ConfinedEngine` requires on the org's lane; then the files
//! feature decides each path by role and grant, exactly as on vox.

use std::sync::Arc;

use architect::http::axum::body::Bytes;
use architect::http::{BodyStream, ByteBody};
use architect_orgs::OrgId;
use architect_permissions::{IdentityResolver, Principal};
use futures_util::StreamExt as _;

use super::{Acting, FilesServer, now_us, ticket};
use crate::error::FilesFault;
use crate::id::{RootId, UploadId, VersionId};
use crate::model::{Entry, OnConflict, Received, UploadPlan, UploadProgress, UploadSpec};
use crate::path::RootPath;
use crate::rendition::{RenditionKind, RenditionSet};
use crate::service::FilesService as _;
use crate::web::FilesWeb;

/// What a request may do: read (a ticket is enough), or anything.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    Read,
    Write,
}

/// A [`FilesServer`] over HTTP.
///
/// ```ignore
/// let web = WebFiles::new(files.clone(), gate.identity_resolver());
/// let app = architect::layers![architect_files::web::Service].provide_http(&web);
/// ```
#[derive(Clone)]
pub struct WebFiles {
    files: FilesServer,
    identity: Arc<dyn IdentityResolver>,
}

impl WebFiles {
    /// `files`, its callers resolved by `identity` (the gate's).
    pub fn new(files: FilesServer, identity: Arc<dyn IdentityResolver>) -> Self {
        Self { files, identity }
    }

    /// The service for this request's caller in `org`, for `path` (a
    /// ticket covers only its own) when it reads.
    async fn acting(
        &self,
        org: &str,
        token: &str,
        need: Need,
        at: Option<(RootId, &RootPath)>,
    ) -> Result<Acting, FilesFault> {
        let org_id = OrgId::new(org);
        if ticket::is_ticket(token) {
            let claims = self
                .files
                .tickets()
                .open(token, now_us())
                .ok_or(FilesFault::Unauthenticated)?;
            let covered = at.is_some_and(|(root, path)| claims.covers(org, root, path));
            if need != Need::Read || !covered {
                // A ticket reads what it was minted for, and nothing else.
                return Err(FilesFault::Unauthenticated);
            }
            let who = claims.principal().ok_or(FilesFault::Unauthenticated)?;
            return self.member(org_id, who);
        }
        let token = Some(token).filter(|t| !t.is_empty());
        match self.identity.resolve(token).await {
            Principal::Anonymous => Err(FilesFault::Unauthenticated),
            who => self.member(org_id, who),
        }
    }

    fn member(&self, org: OrgId, who: Principal) -> Result<Acting, FilesFault> {
        if !self.files.is_member(&org, &who) {
            return Err(FilesFault::denied(
                "use this org's files (not a member)",
                RootPath::root(),
            ));
        }
        Ok(self.files.acting(org, who))
    }
}

impl FilesWeb for WebFiles {
    async fn content(
        &self,
        org: String,
        root: RootId,
        path: String,
        version: Option<VersionId>,
        token: String,
    ) -> Result<ByteBody, FilesFault> {
        let path = RootPath::parse(&path)?;
        let acting = self
            .acting(&org, &token, Need::Read, Some((root, &path)))
            .await?;
        let (content, _) = acting.locate(root, &path, version).await?;
        let blob = acting.blob_path(&content)?;
        ByteBody::file(blob, mime_of(&path))
            .await
            .map_err(super::io)
    }

    async fn rendition(
        &self,
        org: String,
        root: RootId,
        kind: RenditionKind,
        path: String,
        version: Option<VersionId>,
        token: String,
    ) -> Result<ByteBody, FilesFault> {
        let path = RootPath::parse(&path)?;
        let acting = self
            .acting(&org, &token, Need::Read, Some((root, &path)))
            .await?;
        let (blob, _) = acting.rendition(root, &path, version, kind).await?;
        ByteBody::file(blob, kind.mime()).await.map_err(super::io)
    }

    async fn renditions(
        &self,
        org: String,
        root: RootId,
        path: String,
        version: Option<VersionId>,
        token: String,
    ) -> Result<RenditionSet, FilesFault> {
        let path = RootPath::parse(&path)?;
        self.acting(&org, &token, Need::Read, Some((root, &path)))
            .await?
            .renditions(root, path, version)
            .await
    }

    async fn begin_upload(
        &self,
        org: String,
        spec: UploadSpec,
        token: String,
    ) -> Result<UploadPlan, FilesFault> {
        self.acting(&org, &token, Need::Write, None)
            .await?
            .begin_upload(spec)
            .await
    }

    async fn upload_progress(
        &self,
        org: String,
        upload: UploadId,
        token: String,
    ) -> Result<UploadProgress, FilesFault> {
        self.acting(&org, &token, Need::Write, None)
            .await?
            .upload_progress(upload)
            .await
    }

    async fn send_bytes(
        &self,
        org: String,
        upload: UploadId,
        offset: u64,
        token: String,
        body: BodyStream,
    ) -> Result<Received, FilesFault> {
        let acting = self.acting(&org, &token, Need::Write, None).await?;
        let chunks = body.scan(offset, |at, chunk: std::io::Result<Bytes>| {
            let item = chunk
                .map(|bytes| {
                    let start = *at;
                    *at = at.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                    (start, bytes.to_vec())
                })
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::FileTooLarge => FilesFault::invalid(e.to_string()),
                    _ => FilesFault::Io(format!("the upload broke off: {e}")),
                });
            std::future::ready(Some(item))
        });
        acting.receive(upload, chunks).await
    }

    async fn complete_upload(
        &self,
        org: String,
        upload: UploadId,
        on_conflict: OnConflict,
        token: String,
    ) -> Result<Entry, FilesFault> {
        self.acting(&org, &token, Need::Write, None)
            .await?
            .complete_upload(upload, on_conflict)
            .await
    }

    async fn abort_upload(
        &self,
        org: String,
        upload: UploadId,
        token: String,
    ) -> Result<(), FilesFault> {
        self.acting(&org, &token, Need::Write, None)
            .await?
            .abort_upload(upload)
            .await
    }
}

/// The content type a file is served as, by its extension. Only types a
/// browser shows inertly: a page or an SVG of a member's would run its
/// scripts on this origin, so those (and anything unknown) are bytes.
fn mime_of(path: &RootPath) -> &'static str {
    let ext = path
        .name()
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "aif" | "aiff" => "audio/aiff",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain; charset=utf-8",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}
