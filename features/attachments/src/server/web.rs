//! [`WebAttachments`] (feature `http`): the [`AttachmentsWeb`] byte face
//! of an [`AttachmentsServer`].
//!
//! The HTTP face is not behind the vox gate, so this resolves the caller
//! itself: a session token through the gate's own [`IdentityResolver`],
//! or a read ticket for the parent this server signed. Then the parent
//! decides, as on vox: its `read`, and only bytes it points at.

use std::sync::Arc;

use architect::http::ByteBody;
use architect_files::RenditionKind;
use architect_files::server::ticket_principal;
use architect_orgs::OrgId;
use architect_permissions::{IdentityResolver, Principal};

use super::acting::{Claims, TICKET_PREFIX};
use super::{Acting, AttachmentsServer, io, now_us};
use crate::model::{AttachFault, Parent};
use crate::web::AttachmentsWeb;

/// An [`AttachmentsServer`] over HTTP.
///
/// ```ignore
/// let web = WebAttachments::new(attachments.clone(), gate.identity_resolver());
/// let app = architect::layers![architect_attachments::web::Service].provide_http(&web);
/// ```
#[derive(Clone)]
pub struct WebAttachments {
    server: AttachmentsServer,
    identity: Arc<dyn IdentityResolver>,
}

impl WebAttachments {
    /// `server`, its callers resolved by `identity` (the gate's).
    pub fn new(server: AttachmentsServer, identity: Arc<dyn IdentityResolver>) -> Self {
        Self { server, identity }
    }

    /// The service for this request's caller in `org`. A ticket reads
    /// only the parent it was minted for.
    async fn acting(&self, org: &str, parent: &Parent, token: &str) -> Result<Acting, AttachFault> {
        let org_id = OrgId::new(org);
        if token.starts_with(TICKET_PREFIX) {
            let claims: Claims = self
                .server
                .tickets
                .unseal(TICKET_PREFIX, token)
                .filter(|c: &Claims| c.expires_us > now_us())
                .ok_or(AttachFault::Unauthenticated)?;
            if claims.org != org || claims.parent != parent.to_string() {
                return Err(AttachFault::Unauthenticated);
            }
            let who = ticket_principal(&claims.who).ok_or(AttachFault::Unauthenticated)?;
            return Ok(self.server.acting(org_id, who));
        }
        let token = Some(token).filter(|t| !t.is_empty());
        match self.identity.resolve(token).await {
            Principal::Anonymous => Err(AttachFault::Unauthenticated),
            who => Ok(self.server.acting(org_id, who)),
        }
    }
}

impl AttachmentsWeb for WebAttachments {
    async fn content(
        &self,
        org: String,
        parent: String,
        hash: String,
        token: String,
    ) -> Result<ByteBody, AttachFault> {
        let parent = Parent::parse(&parent)?;
        let acting = self.acting(&org, &parent, &token).await?;
        let (path, attachment, _) = acting.blob(&parent, &hash, None).await?;
        ByteBody::file(path, inert(&attachment.mime))
            .await
            .map_err(io)
    }

    async fn rendition(
        &self,
        org: String,
        parent: String,
        hash: String,
        kind: RenditionKind,
        token: String,
    ) -> Result<ByteBody, AttachFault> {
        let parent = Parent::parse(&parent)?;
        let acting = self.acting(&org, &parent, &token).await?;
        let (path, _, _) = acting.blob(&parent, &hash, Some(kind)).await?;
        ByteBody::file(path, kind.mime()).await.map_err(io)
    }
}

/// The content type an attachment is served as: its own when a browser
/// shows it inertly. A page or an SVG a member attached would run its
/// scripts on this origin, so those (and anything unknown) are bytes.
fn inert(mime: &str) -> &'static str {
    const SHOWN: &[&str] = &[
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/avif",
        "application/pdf",
        "video/mp4",
        "video/quicktime",
        "video/webm",
        "audio/mp4",
        "audio/mpeg",
        "audio/wav",
        "audio/flac",
        "audio/ogg",
        "audio/opus",
        "audio/aiff",
        "text/plain",
        "application/json",
    ];
    let bare = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match SHOWN.iter().find(|m| **m == bare) {
        Some(&"text/plain") => "text/plain; charset=utf-8",
        Some(m) => m,
        None => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::inert;

    #[test]
    fn only_inert_types_are_served_as_themselves() {
        assert_eq!(inert("image/PNG"), "image/png");
        assert_eq!(inert("application/pdf"), "application/pdf");
        assert_eq!(
            inert("text/plain; charset=latin1"),
            "text/plain; charset=utf-8"
        );
        assert_eq!(inert("text/html"), "application/octet-stream");
        assert_eq!(inert("image/svg+xml"), "application/octet-stream");
        assert_eq!(inert(""), "application/octet-stream");
    }
}
