//! A link to a live document: what one person hands another so they can
//! join it.
//!
//! `fts-doc:<endpoint id>/<doc id>?key=<key>` names the node serving the
//! document (its iroh endpoint id, 64 hex digits), the document, and the
//! key the node minted for this share ([`crate::p2p::Shares`]). The
//! endpoint id is enough to dial the node: on the same network its
//! address is found there, elsewhere through iroh's relays. The key is
//! the credential: whoever holds the link may edit that one document,
//! until the node revokes it.
//!
//! Parsing is wasm-clean, so a web page can read a link it was handed.
//! [`DocLink::bearer`] is what the joiner presents on its `DocSync`
//! calls ([`LinkKey`] puts it there).

use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

/// What a document link starts with.
pub const SCHEME: &str = "fts-doc:";

/// What a joiner's bearer token starts with: the rest is the link's key.
pub const LINK_BEARER_PREFIX: &str = "link:";

/// A link to one live document on one node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocLink {
    /// The serving node's iroh endpoint id (lowercase hex).
    pub endpoint: String,
    pub doc_id: Uuid,
    /// The share's key, minted by the serving node.
    pub key: String,
}

/// Why a string is not a [`DocLink`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("not a document link (they start with `{SCHEME}`)")]
    NotALink,
    #[error("the link's node is not an endpoint id")]
    Endpoint,
    #[error("the link's document is not an id")]
    DocId,
    #[error("the link has no key")]
    Key,
}

impl DocLink {
    /// The bearer token a joiner presents for this link.
    #[must_use]
    pub fn bearer(&self) -> String {
        format!("{LINK_BEARER_PREFIX}{}", self.key)
    }
}

impl fmt::Display for DocLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{SCHEME}{}/{}?key={}",
            self.endpoint, self.doc_id, self.key
        )
    }
}

impl FromStr for DocLink {
    type Err = LinkError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.trim().strip_prefix(SCHEME).ok_or(LinkError::NotALink)?;
        let (endpoint, rest) = rest.split_once('/').ok_or(LinkError::NotALink)?;
        if !is_endpoint_id(endpoint) {
            return Err(LinkError::Endpoint);
        }
        let (doc, query) = rest.split_once('?').ok_or(LinkError::Key)?;
        let doc_id = Uuid::parse_str(doc).map_err(|_| LinkError::DocId)?;
        let key = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("key="))
            .filter(|k| is_key(k))
            .ok_or(LinkError::Key)?;
        Ok(Self {
            endpoint: endpoint.to_ascii_lowercase(),
            doc_id,
            key: key.to_owned(),
        })
    }
}

/// An iroh endpoint id as text: 32 bytes, hex.
fn is_endpoint_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A share key: what [`crate::p2p::Shares`] mints, or anything shaped
/// like it (letters and digits, at most 128).
fn is_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Client middleware presenting a link's key on every call.
///
/// It sends `authorization: Bearer link:<key>`, so the serving node
/// resolves the caller as the link's guest. Attach it to the
/// `DocSyncClient` dialled to the link's node:
/// `client.with_middleware(LinkKey::new(&link))`.
#[cfg(feature = "vox")]
#[derive(Debug, Clone)]
pub struct LinkKey {
    bearer: String,
}

#[cfg(feature = "vox")]
impl LinkKey {
    #[must_use]
    pub fn new(link: &DocLink) -> Self {
        Self {
            bearer: format!("Bearer {}", link.bearer()),
        }
    }
}

#[cfg(feature = "vox")]
impl vox::ClientMiddleware for LinkKey {
    fn pre<'a>(
        &'a self,
        _context: &'a vox::ClientContext<'a>,
        request: &'a mut vox::ClientRequest<'_, 'a>,
    ) -> vox::BoxMiddlewareFuture<'a> {
        Box::pin(async move {
            // `architect::permissions_gate::AUTHORIZATION_METADATA_KEY`,
            // which is native only; a browser joins too.
            request.push_string_metadata("authorization", self.bearer.clone());
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn link() -> DocLink {
        DocLink {
            endpoint: "ab".repeat(32),
            doc_id: Uuid::from_u128(7),
            key: "0123abcd".into(),
        }
    }

    #[test]
    fn a_link_reads_back_as_itself() {
        let text = link().to_string();
        assert!(text.starts_with("fts-doc:abab"));
        assert_eq!(text.parse::<DocLink>().unwrap(), link());
    }

    #[test]
    fn what_is_not_a_link_says_why() {
        let endpoint = "ab".repeat(32);
        let doc = Uuid::from_u128(7);
        assert_eq!("https://x".parse::<DocLink>(), Err(LinkError::NotALink));
        assert_eq!(
            format!("fts-doc:abc/{doc}?key=k").parse::<DocLink>(),
            Err(LinkError::Endpoint)
        );
        assert_eq!(
            format!("fts-doc:{endpoint}/nope?key=k").parse::<DocLink>(),
            Err(LinkError::DocId)
        );
        assert_eq!(
            format!("fts-doc:{endpoint}/{doc}").parse::<DocLink>(),
            Err(LinkError::Key)
        );
        assert_eq!(
            format!("fts-doc:{endpoint}/{doc}?key=a/b").parse::<DocLink>(),
            Err(LinkError::Key)
        );
    }
}
