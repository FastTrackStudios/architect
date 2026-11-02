//! The HTTP byte face: an org's files for a browser — bytes it reads by
//! range (`<video>`, `<audio>`, `<img>`, `fetch`), renditions, and a
//! resumable upload in plain `PUT`s.
//!
//! ```text
//! GET    /files/{org}/{root}/content/{*path}?version=&token=          the bytes, ranged
//! GET    /files/{org}/{root}/rendition/{kind}/{*path}?version=&token= a rendition, ranged
//! GET    /files/{org}/{root}/renditions/{*path}?version=&token=       what renditions there are
//! POST   /files/{org}/uploads                     {spec}              begin: what to send
//! PUT    /files/{org}/uploads/{upload}?offset=    bytes               send some, from `offset`
//! GET    /files/{org}/uploads/{upload}                                where to resume
//! POST   /files/{org}/uploads/{upload}/complete   {on_conflict}       land it
//! DELETE /files/{org}/uploads/{upload}                                abandon it
//! ```
//!
//! `{org}` is the org's id. Every route acts for `token`: a session
//! (`Authorization: Bearer …`, or `?token=`), or a read ticket minted
//! over the org's lane ([`crate::FilesService::ticket`]) for an element
//! that cannot send a header. The same rules hold as on vox: the caller's
//! role in the org and their grants decide each path, and outside them a
//! path is absent. The server side is `server::WebFiles` (feature
//! `http`); the URL builders here are wasm-clean.

use architect::http::{BodyStream, ByteBody, QueryString, encode_rest, encode_segment};
use architect_orgs::OrgId;

use crate::error::FilesFault;
use crate::id::{RootId, UploadId, VersionId};
use crate::model::{Entry, OnConflict, Received, UploadPlan, UploadProgress, UploadSpec};
use crate::path::RootPath;
use crate::rendition::{RenditionKind, RenditionSet};

#[architect::service(path = "files/{org}")]
pub trait FilesWeb {
    /// A file's bytes — its current version, or `version` (needs
    /// `History`) — answering `Range` with `206`.
    #[http(get, path = "{root}/content/{*path}")]
    async fn content(
        &self,
        org: String,
        root: RootId,
        path: String,
        version: Option<VersionId>,
        token: String,
    ) -> Result<ByteBody, FilesFault>;

    /// One rendition of a version, answering `Range`. One not made yet
    /// is `503 rendition_pending` (and is asked for); one that does not
    /// apply, or failed, is `404 no_rendition`.
    #[http(get, path = "{root}/rendition/{kind}/{*path}")]
    async fn rendition(
        &self,
        org: String,
        root: RootId,
        kind: RenditionKind,
        path: String,
        version: Option<VersionId>,
        token: String,
    ) -> Result<ByteBody, FilesFault>;

    /// A version's renditions and where each one is.
    #[http(get, path = "{root}/renditions/{*path}")]
    async fn renditions(
        &self,
        org: String,
        root: RootId,
        path: String,
        version: Option<VersionId>,
        token: String,
    ) -> Result<RenditionSet, FilesFault>;

    /// Begin an upload: what to send (nothing, when the org holds these
    /// bytes).
    #[http(post, path = "uploads")]
    async fn begin_upload(
        &self,
        org: String,
        spec: UploadSpec,
        token: String,
    ) -> Result<UploadPlan, FilesFault>;

    /// What an upload still needs: where a client resumes.
    #[http(get, path = "uploads/{upload}")]
    async fn upload_progress(
        &self,
        org: String,
        upload: UploadId,
        token: String,
    ) -> Result<UploadProgress, FilesFault>;

    /// Bytes of an upload, from `offset` on: the body is written there,
    /// and what has arrived counts even when the body breaks off.
    #[http(put, path = "uploads/{upload}", limit = "1GiB")]
    async fn send_bytes(
        &self,
        org: String,
        upload: UploadId,
        offset: u64,
        token: String,
        body: BodyStream,
    ) -> Result<Received, FilesFault>;

    /// Land the upload.
    #[http(post, path = "uploads/{upload}/complete")]
    async fn complete_upload(
        &self,
        org: String,
        upload: UploadId,
        on_conflict: OnConflict,
        token: String,
    ) -> Result<Entry, FilesFault>;

    /// Abandon it.
    #[http(delete, path = "uploads/{upload}")]
    async fn abort_upload(
        &self,
        org: String,
        upload: UploadId,
        token: String,
    ) -> Result<(), FilesFault>;
}

/// Where a file's bytes are, under `base` (the server's origin, or `""`
/// for the page's own): what a `<video src>` or `fetch` takes. `token` is
/// a [`crate::Ticket`]'s for an element that cannot send a header.
#[must_use]
pub fn content_url(
    base: &str,
    org: &OrgId,
    root: RootId,
    path: &RootPath,
    version: Option<VersionId>,
    token: Option<&str>,
) -> String {
    url(base, org, root, "content", path, version, token)
}

/// Where a rendition of a file's version is, under `base`.
#[must_use]
pub fn rendition_url(
    base: &str,
    org: &OrgId,
    root: RootId,
    kind: RenditionKind,
    path: &RootPath,
    version: Option<VersionId>,
    token: Option<&str>,
) -> String {
    url(
        base,
        org,
        root,
        &format!("rendition/{}", kind.tag()),
        path,
        version,
        token,
    )
}

fn url(
    base: &str,
    org: &OrgId,
    root: RootId,
    what: &str,
    path: &RootPath,
    version: Option<VersionId>,
    token: Option<&str>,
) -> String {
    let mut query = QueryString::new();
    query.push_opt("version", version.as_ref());
    query.push_opt("token", token.filter(|t| !t.is_empty()).as_ref());
    query.after(&format!(
        "{}/files/{}/{root}/{what}/{}",
        base.trim_end_matches('/'),
        encode_segment(&org.0),
        encode_rest(&path.as_str()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn urls_encode_the_path_and_carry_the_version_and_token() {
        let org = OrgId::new("fts");
        let root = RootId::new(Uuid::nil());
        let path = RootPath::parse("Review/cut 2.mov").unwrap();
        assert_eq!(
            content_url("https://x.test/", &org, root, &path, None, None),
            format!("https://x.test/files/fts/{root}/content/Review/cut%202.mov")
        );
        let version = VersionId::new(Uuid::nil());
        assert_eq!(
            rendition_url(
                "",
                &org,
                root,
                RenditionKind::Proxy,
                &path,
                Some(version),
                Some("t+1")
            ),
            format!(
                "/files/fts/{root}/rendition/proxy/Review/cut%202.mov?version={version}&token=t%2B1"
            )
        );
    }
}
