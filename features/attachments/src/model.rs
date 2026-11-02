//! What an attachment is, what it hangs off, and what goes wrong.

use std::fmt;

use architect_files::{FilesFault, RenditionKind};
use facet::Facet;
use thiserror::Error;
use uuid::Uuid;

/// What an attachment hangs off: a co-edited document, or a record (a
/// row of the app's: a task, a contact, an invoice).
///
/// Access follows it: whoever may read the parent reads its attachments,
/// whoever may write it attaches and detaches. It is checked as the
/// resource [`Parent::resource`] names, against the org's permission
/// engine (the gate's), so a space's members (`architect-spaces`) and a
/// document's own permissions (`crdt::registry::DocPermissions`, the same
/// `doc/<id>`) hold here too.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(u8)]
pub enum Parent {
    /// A CRDT document: it lists its attachments itself ([`crate::doc`]),
    /// so attaching works offline and syncs with it.
    Doc { id: Uuid },
    /// A record of `kind` (`task`, `contact`): the server keeps its list.
    Record { kind: String, id: String },
}

impl Parent {
    #[must_use]
    pub const fn doc(id: Uuid) -> Self {
        Self::Doc { id }
    }

    #[must_use]
    pub fn record(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self::Record {
            kind: kind.into(),
            id: id.into(),
        }
    }

    /// The resource the parent is checked as, org-relative: `doc/<id>`
    /// for a document, `<kind>/<id>` for a record.
    #[must_use]
    pub fn resource(&self) -> String {
        match self {
            Self::Doc { id } => format!("doc/{id}"),
            Self::Record { kind, id } => format!("{kind}/{id}"),
        }
    }

    /// Whether it names something: a record's kind is a lowercase word
    /// (not `doc`), its id is not empty and holds no `/`.
    ///
    /// # Errors
    ///
    /// [`AttachFault::Invalid`] saying why.
    pub fn validate(&self) -> Result<(), AttachFault> {
        let Self::Record { kind, id } = self else {
            return Ok(());
        };
        let word = !kind.is_empty()
            && kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        if !word || kind == "doc" {
            return Err(AttachFault::invalid(format!(
                "a record's kind is a lowercase word other than `doc`, not {kind:?}"
            )));
        }
        if id.is_empty() || id.contains('/') {
            return Err(AttachFault::invalid(format!(
                "a record's id is not empty and holds no `/`, not {id:?}"
            )));
        }
        Ok(())
    }

    /// From [`Parent`]'s text (`doc:<uuid>`, `<kind>:<id>`): what a URL
    /// carries.
    ///
    /// # Errors
    ///
    /// [`AttachFault::Invalid`] for anything else.
    pub fn parse(text: &str) -> Result<Self, AttachFault> {
        let (kind, id) = text
            .split_once(':')
            .ok_or_else(|| AttachFault::invalid(format!("not a parent: {text:?}")))?;
        let parent = if kind == "doc" {
            Uuid::parse_str(id)
                .map(Self::doc)
                .map_err(|e| AttachFault::invalid(format!("not a document id: {e}")))?
        } else {
            Self::record(kind, id)
        };
        parent.validate()?;
        Ok(parent)
    }
}

/// `doc:<uuid>` or `<kind>:<id>`.
impl fmt::Display for Parent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Doc { id } => write!(f, "doc:{id}"),
            Self::Record { kind, id } => write!(f, "{kind}:{id}"),
        }
    }
}

/// One file attached to a parent. Its bytes are the blob `hash` names,
/// stored once however many parents attach them; a parent attaches the
/// same bytes once.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Attachment {
    /// The bytes' sha256, 64 lowercase hex digits.
    pub hash: String,
    /// The file's name, as the person attached it (`brief.pdf`).
    pub name: String,
    /// Its content type (`image/png`). The byte face serves only inert
    /// types as themselves.
    pub mime: String,
    pub size: u64,
    /// Microseconds since the Unix epoch.
    pub added_us: i64,
    /// `user:<id>`, as a ticket writes a principal; `None` when unknown.
    pub added_by: Option<String>,
}

impl Attachment {
    /// Whether `hash` is a sha256 and the name is a file's name.
    ///
    /// # Errors
    ///
    /// [`AttachFault::Invalid`] saying why.
    pub fn validate(&self) -> Result<(), AttachFault> {
        check_hash(&self.hash)?;
        if self.name.is_empty() || self.name.contains(['/', '\\', '\0']) {
            return Err(AttachFault::invalid(format!(
                "an attachment's name is a file's name, not {:?}",
                self.name
            )));
        }
        Ok(())
    }
}

/// Whether `hash` is 64 lowercase hex digits.
///
/// # Errors
///
/// [`AttachFault::Invalid`].
pub fn check_hash(hash: &str) -> Result<(), AttachFault> {
    if hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(AttachFault::invalid(format!("not a sha256: {hash:?}")))
    }
}

/// How far the server has an attachment's bytes: what a device asks
/// before sending (and after a dropped link, to resume).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Upload {
    /// Bytes the server holds, from the start. Equal to `size`: it has
    /// them all, send nothing.
    pub received: u64,
    pub size: u64,
}

impl Upload {
    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.received >= self.size
    }
}

/// What the org's attachments hold, and may.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct AttachUsage {
    /// Bytes of the distinct blobs the org's attachments hold.
    pub used: u64,
    pub quota: Option<u64>,
}

/// What to read: an attachment's bytes (or a rendition's), from
/// `offset` to the end.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ReadRequest {
    pub parent: Parent,
    pub hash: String,
    pub rendition: Option<RenditionKind>,
    pub offset: u64,
}

impl ReadRequest {
    /// The bytes themselves, from `offset`.
    #[must_use]
    pub fn bytes(parent: Parent, hash: impl Into<String>, offset: u64) -> Self {
        Self {
            parent,
            hash: hash.into(),
            rendition: None,
            offset,
        }
    }
}

/// A read's result, after its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct ReadDone {
    /// The whole blob's length (the rendition's, when one was read).
    pub total: u64,
    /// Bytes sent.
    pub sent: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet, Error)]
#[repr(u8)]
pub enum AttachFault {
    /// The call carries no identity to act for.
    #[error("no identity: sign in")]
    Unauthenticated,
    /// The call is in no organization.
    #[error("this call is in no organization")]
    NoOrg,
    /// The caller may not do this to the parent.
    #[error("not permitted: {action} on {parent}")]
    Denied { action: String, parent: String },
    /// No such attachment on the parent — or none the caller may see: a
    /// hash its parent does not point at is absent, whoever asks.
    #[error("no attachment {hash} on {parent}")]
    NotFound { parent: String, hash: String },
    /// The server cannot see this document (no document source, or not
    /// one it hosts).
    #[error("no such document: {0}")]
    NoDoc(String),
    #[error("{0}")]
    Invalid(String),
    /// The bytes sent are not the ones named: the upload starts again.
    #[error("the bytes sent hash to {got}, not {expected}")]
    Mismatch { expected: String, got: String },
    /// Bytes sent from somewhere other than where the upload stands.
    #[error("the upload stands at {received}, not {offset}")]
    Offset { received: u64, offset: u64 },
    /// The org's attachments would outgrow its quota.
    #[error("over quota: {used} of {quota} bytes used, {adding} more asked")]
    QuotaExceeded { used: u64, quota: u64, adding: u64 },
    /// The rendition is being made: ask again soon.
    #[error("the {0} rendition is being made")]
    RenditionPending(RenditionKind),
    /// There is no such rendition: its kind does not apply, it failed, or
    /// the server makes none.
    #[error("no {kind} rendition: {reason}")]
    NoRendition { kind: RenditionKind, reason: String },
    #[error("io: {0}")]
    Io(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl AttachFault {
    #[must_use]
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid(reason.into())
    }

    #[must_use]
    pub fn denied(action: &str, parent: &Parent) -> Self {
        Self::Denied {
            action: action.to_owned(),
            parent: parent.to_string(),
        }
    }

    #[must_use]
    pub fn not_found(parent: &Parent, hash: &str) -> Self {
        Self::NotFound {
            parent: parent.to_string(),
            hash: hash.to_owned(),
        }
    }

    /// Whether retrying unchanged could succeed.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Io(_) | Self::RenditionPending(_))
    }
}

/// A rendition's fault, from the files feature's renditions.
impl From<FilesFault> for AttachFault {
    fn from(fault: FilesFault) -> Self {
        match fault {
            FilesFault::RenditionPending(kind) => Self::RenditionPending(kind),
            FilesFault::NoRendition { kind, reason } => Self::NoRendition { kind, reason },
            FilesFault::Io(e) => Self::Io(e),
            other => Self::Internal(other.to_string()),
        }
    }
}

/// How a fault reads on the HTTP face (`crate::web`).
impl architect::http::HttpError for AttachFault {
    fn status(&self) -> u16 {
        match self {
            Self::Unauthenticated => 401,
            Self::Denied { .. } => 403,
            Self::NoOrg | Self::NotFound { .. } | Self::NoDoc(_) | Self::NoRendition { .. } => 404,
            Self::Invalid(_) | Self::Mismatch { .. } => 400,
            Self::Offset { .. } => 409,
            Self::QuotaExceeded { .. } => 507,
            Self::RenditionPending(_) | Self::Io(_) => 503,
            Self::Internal(_) => 500,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::NoOrg => "no_org",
            Self::Denied { .. } => "denied",
            Self::NotFound { .. } => "not_found",
            Self::NoDoc(_) => "no_doc",
            Self::Invalid(_) => "invalid",
            Self::Mismatch { .. } => "mismatch",
            Self::Offset { .. } => "offset",
            Self::QuotaExceeded { .. } => "quota_exceeded",
            Self::RenditionPending(_) => "rendition_pending",
            Self::NoRendition { .. } => "no_rendition",
            Self::Io(_) => "io",
            Self::Internal(_) => "internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_reads_back_from_its_text_and_names_its_resource() {
        let doc = Parent::doc(Uuid::from_u128(7));
        assert_eq!(Parent::parse(&doc.to_string()).unwrap(), doc);
        assert_eq!(doc.resource(), format!("doc/{}", Uuid::from_u128(7)));
        let task = Parent::record("task", "T-1:a");
        assert_eq!(Parent::parse("task:T-1:a").unwrap(), task);
        assert_eq!(task.resource(), "task/T-1:a");
        for bad in ["doc:nope", "Task:1", "task:", "task:a/b", "nothing", ":1"] {
            assert!(Parent::parse(bad).is_err(), "{bad}");
        }
        assert!(Parent::record("doc", "x").validate().is_err());
    }

    #[test]
    fn an_attachment_names_a_sha256_and_a_file() {
        let mut a = Attachment {
            hash: "a".repeat(64),
            name: "brief.pdf".into(),
            mime: "application/pdf".into(),
            size: 3,
            added_us: 0,
            added_by: None,
        };
        assert!(a.validate().is_ok());
        a.name = "../x".into();
        assert!(a.validate().is_err());
        a.name = "x".into();
        a.hash = "A".repeat(64);
        assert!(a.validate().is_err());
    }
}
