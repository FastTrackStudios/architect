//! The HTTP byte face: an attachment's bytes, or a rendition of them, for
//! a browser's `<img>`, `<video>`, `<audio>`, `<iframe>` or `fetch`,
//! answering `Range` with `206`.
//!
//! ```text
//! GET /attachments/{org}/{parent}/{hash}?token=                   the bytes, ranged
//! GET /attachments/{org}/{parent}/{hash}/rendition/{kind}?token=  a rendition, ranged
//! ```
//!
//! `{parent}` is the parent's text (`doc:<uuid>`, `task:T-1`). Every route
//! acts for `token`: a session (`Authorization: Bearer …`, or `?token=`)
//! or a read ticket for the parent minted over the org's lane
//! ([`crate::AttachmentsService::ticket`]). The rules are the lane's: the
//! parent's `read`, and only bytes the parent points at. The server side
//! is `server::WebAttachments` (feature `http`); the URL builders here are
//! wasm-clean.

use architect::http::{ByteBody, QueryString, encode_segment};
use architect_files::RenditionKind;
use architect_orgs::OrgId;

use crate::model::{AttachFault, Parent};

#[architect::service(path = "attachments/{org}")]
pub trait AttachmentsWeb {
    /// An attachment's bytes, answering `Range` with `206`.
    #[http(get, path = "{parent}/{hash}")]
    async fn content(
        &self,
        org: String,
        parent: String,
        hash: String,
        token: String,
    ) -> Result<ByteBody, AttachFault>;

    /// A rendition of them. One not made yet is `503 rendition_pending`
    /// (and is asked for); one that does not apply is `404 no_rendition`.
    #[http(get, path = "{parent}/{hash}/rendition/{kind}")]
    async fn rendition(
        &self,
        org: String,
        parent: String,
        hash: String,
        kind: RenditionKind,
        token: String,
    ) -> Result<ByteBody, AttachFault>;
}

/// Where an attachment's bytes are, under `base` (the server's origin,
/// or `""` for the page's own). `token` is a ticket's, for an element
/// that cannot send a header.
#[must_use]
pub fn content_url(
    base: &str,
    org: &OrgId,
    parent: &Parent,
    hash: &str,
    token: Option<&str>,
) -> String {
    url(base, org, parent, hash, "", token)
}

/// Where a rendition of an attachment is, under `base`.
#[must_use]
pub fn rendition_url(
    base: &str,
    org: &OrgId,
    parent: &Parent,
    hash: &str,
    kind: RenditionKind,
    token: Option<&str>,
) -> String {
    url(
        base,
        org,
        parent,
        hash,
        &format!("/rendition/{}", kind.tag()),
        token,
    )
}

fn url(
    base: &str,
    org: &OrgId,
    parent: &Parent,
    hash: &str,
    tail: &str,
    token: Option<&str>,
) -> String {
    let mut query = QueryString::new();
    query.push_opt("token", token.filter(|t| !t.is_empty()).as_ref());
    query.after(&format!(
        "{}/attachments/{}/{}/{}{tail}",
        base.trim_end_matches('/'),
        encode_segment(&org.0),
        encode_segment(parent),
        encode_segment(&hash),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_encode_the_parent_and_carry_the_token() {
        let org = OrgId::new("fts");
        let hash = "a".repeat(64);
        let task = Parent::record("task", "T 1");
        assert_eq!(
            content_url("https://x.test/", &org, &task, &hash, None),
            format!("https://x.test/attachments/fts/task%3AT%201/{hash}")
        );
        assert_eq!(
            rendition_url("", &org, &task, &hash, RenditionKind::Poster, Some("t+1")),
            format!("/attachments/fts/task%3AT%201/{hash}/rendition/poster?token=t%2B1")
        );
    }
}
