//! What went wrong, in terms a caller can branch on: prose for the
//! person, the variant for the code.

use facet::Facet;
use thiserror::Error;

use crate::id::{GrantId, LabelId, RootId, UploadId, VersionId};
use crate::path::{PathError, RootPath};
use crate::rendition::RenditionKind;

#[derive(Debug, Clone, PartialEq, Eq, Facet, Error)]
#[repr(u8)]
pub enum FilesFault {
    // ── Who and where ──────────────────────────────────────────────
    /// The call carries no identity the files feature acts for (no
    /// gate resolved one, or it is a guest or a peer host).
    #[error("no identity: sign in")]
    Unauthenticated,
    /// The call is in no organization: a lane in no org, and the server
    /// has no default one.
    #[error("this call is in no organization")]
    NoOrg,

    // ── Identity ───────────────────────────────────────────────────
    #[error("no such root: {0}")]
    RootNotFound(RootId),
    /// Nothing at the path — or nothing the caller may see there: outside
    /// every folder they hold, a path is absent, not forbidden.
    #[error("no such path: {0}")]
    PathNotFound(RootPath),
    #[error("no such version: {0}")]
    VersionNotFound(VersionId),
    #[error("no such label: {0}")]
    LabelNotFound(LabelId),
    #[error("no such upload: {0}")]
    UploadNotFound(UploadId),
    #[error("no such grant: {0}")]
    GrantNotFound(GrantId),

    // ── Refusal ────────────────────────────────────────────────────
    /// The destination is taken; the caller chooses what to do
    /// ([`crate::OnConflict`]), never the server.
    #[error("already exists: {path}")]
    Exists { path: RootPath },
    #[error("path is not valid: {0}")]
    BadPath(#[from] PathError),
    #[error("expected a folder: {0}")]
    NotADirectory(RootPath),
    #[error("expected a file: {0}")]
    NotAFile(RootPath),
    /// A move or copy into its own source.
    #[error("cannot put {from} inside itself at {to}")]
    IntoSelf { from: RootPath, to: RootPath },
    #[error("{0}")]
    Invalid(String),
    /// The caller may see the path and may not do this there.
    #[error("not permitted: {action} on {path}")]
    Denied { action: String, path: RootPath },

    // ── Concurrency and limits ─────────────────────────────────────
    /// A save's precondition failed: someone else changed the file first.
    #[error("stale: {path} is no longer {expected}")]
    Stale { path: RootPath, expected: String },
    /// The org's files would outgrow its quota.
    #[error("over quota: {used} of {quota} bytes used, {adding} more asked")]
    QuotaExceeded { used: u64, quota: u64, adding: u64 },
    /// An upload landed before all its bytes arrived.
    #[error("upload {0} is missing bytes")]
    Incomplete(UploadId),

    // ── Below us ───────────────────────────────────────────────────
    #[error("io: {0}")]
    Io(String),
    #[error("internal: {0}")]
    Internal(String),

    // ── Renditions ─────────────────────────────────────────────────
    /// The rendition is being made (or waits its turn): ask again soon.
    #[error("the {0} rendition is being made")]
    RenditionPending(RenditionKind),
    /// There is no such rendition of this version: its kind does not
    /// apply to the source, the transcoder failed on it, or the server
    /// makes no renditions.
    #[error("no {kind} rendition: {reason}")]
    NoRendition { kind: RenditionKind, reason: String },
}

impl FilesFault {
    #[must_use]
    pub fn denied(action: impl Into<String>, path: RootPath) -> Self {
        Self::Denied {
            action: action.into(),
            path,
        }
    }

    #[must_use]
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid(reason.into())
    }

    /// Whether retrying unchanged could succeed.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Io(_) | Self::RenditionPending(_))
    }
}

/// How a fault reads on the HTTP face (`crate::web`).
impl architect::http::HttpError for FilesFault {
    fn status(&self) -> u16 {
        match self {
            Self::Unauthenticated => 401,
            Self::Denied { .. } => 403,
            Self::NoOrg
            | Self::RootNotFound(_)
            | Self::PathNotFound(_)
            | Self::VersionNotFound(_)
            | Self::LabelNotFound(_)
            | Self::UploadNotFound(_)
            | Self::GrantNotFound(_)
            | Self::NoRendition { .. } => 404,
            Self::Exists { .. } | Self::Incomplete(_) => 409,
            Self::Stale { .. } => 412,
            Self::QuotaExceeded { .. } => 507,
            Self::BadPath(_)
            | Self::NotADirectory(_)
            | Self::NotAFile(_)
            | Self::IntoSelf { .. }
            | Self::Invalid(_) => 400,
            Self::RenditionPending(_) | Self::Io(_) => 503,
            Self::Internal(_) => 500,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::NoOrg => "no_org",
            Self::RootNotFound(_) => "root_not_found",
            Self::PathNotFound(_) => "path_not_found",
            Self::VersionNotFound(_) => "version_not_found",
            Self::LabelNotFound(_) => "label_not_found",
            Self::UploadNotFound(_) => "upload_not_found",
            Self::GrantNotFound(_) => "grant_not_found",
            Self::Exists { .. } => "exists",
            Self::BadPath(_) => "bad_path",
            Self::NotADirectory(_) => "not_a_directory",
            Self::NotAFile(_) => "not_a_file",
            Self::IntoSelf { .. } => "into_self",
            Self::Invalid(_) => "invalid",
            Self::Denied { .. } => "denied",
            Self::Stale { .. } => "stale",
            Self::QuotaExceeded { .. } => "quota_exceeded",
            Self::Incomplete(_) => "incomplete",
            Self::Io(_) => "io",
            Self::Internal(_) => "internal",
            Self::RenditionPending(_) => "rendition_pending",
            Self::NoRendition { .. } => "no_rendition",
        }
    }
}
