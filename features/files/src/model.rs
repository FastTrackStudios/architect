//! The wire's shapes: roots, entries, versions and their labels, the
//! write and upload lanes' requests and receipts, and the event stream.

use facet::Facet;

use crate::access::Grant;
use crate::id::{ContentId, LabelId, RootId, UploadId, VersionId};
use crate::path::RootPath;

/// A root: a tree of files and folders, in one org.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Root {
    pub id: RootId,
    /// What a person sees.
    pub name: String,
    /// Where it is in the org's files (`signal/impulse-responses`): the
    /// key an app finds its root by again ([`crate::FilesService::ensure_root`]).
    pub dir: String,
    /// Who made it (a user id, or `service:<name>`).
    pub created_by: String,
    /// Microseconds since the Unix epoch, server clock.
    pub created_us: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum EntryKind {
    File,
    Directory,
}

/// One file or folder, as it is now. Carries no bytes.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Entry {
    pub root_id: RootId,
    pub path: RootPath,
    pub kind: EntryKind,
    /// A file's length; 0 for a folder.
    pub size: u64,
    /// A file's bytes (its etag); `None` for a folder.
    pub content: Option<ContentId>,
    /// A file's current version; `None` for a folder.
    pub version: Option<VersionId>,
    pub modified_us: i64,
}

/// One version of a file: what one save, copy or restore wrote. A delete
/// is a version too (`content: None`), so "recently deleted" is history
/// and a deleted file can be restored.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Version {
    pub id: VersionId,
    /// `None`: the file was deleted here.
    pub content: Option<ContentId>,
    pub size: u64,
    /// Who wrote it.
    pub by: String,
    pub at_us: i64,
}

/// A name a person gave one version of one file. Unique per file.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct VersionLabel {
    pub id: LabelId,
    pub root_id: RootId,
    pub path: RootPath,
    pub version: VersionId,
    pub name: String,
    pub by: String,
    pub at_us: i64,
}

/// An org's space: the bytes its live files hold (each file's current
/// version, counted once per path) and its quota, if it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Usage {
    pub used: u64,
    pub quota: Option<u64>,
}

/// What to do when a destination is taken. No default: the caller
/// chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum OnConflict {
    /// Refuse with [`crate::FilesFault::Exists`].
    Fail,
    /// Land beside it under a derived name (`take (2).wav`).
    KeepBoth,
    /// Replace it — a new version of the file, the old one kept in its
    /// history.
    Replace,
    /// Leave it, drop the incoming.
    KeepExisting,
}

/// A precondition on what occupies a save's destination.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum Expect {
    /// Nothing is there.
    Absent,
    /// Exactly this content is there: replace what I last read, and
    /// nothing newer.
    Content(ContentId),
}

/// A move or copy.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Relocation {
    pub from: RootPath,
    pub to: RootPath,
}

/// What happened to one path of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Outcome {
    pub path: RootPath,
    /// Where it landed: the destination, or the derived name under
    /// [`OnConflict::KeepBoth`]; `None` for a delete or a skip.
    pub landed_at: Option<RootPath>,
    /// Nothing was done ([`OnConflict::KeepExisting`]).
    pub skipped: bool,
}

/// A write batch's result. A batch applies all of it or none.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct WriteReceipt {
    pub root_id: RootId,
    pub outcomes: Vec<Outcome>,
}

/// A half-open byte range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// What a client wants to put where.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct UploadSpec {
    pub root_id: RootId,
    pub path: RootPath,
    pub size: u64,
    /// The bytes' sha256, when the client knows it: content the org
    /// already holds is then not sent at all, and what arrives is checked
    /// against it.
    pub content: Option<ContentId>,
    /// What the caller believes is at `path` now; checked when the
    /// upload lands. `None` asserts nothing.
    pub expect: Option<Expect>,
}

/// The server's answer to [`UploadSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct UploadPlan {
    pub upload_id: UploadId,
    /// What to send. Empty: nothing — the org holds these bytes already
    /// (or the file is empty), complete now.
    pub needed: Vec<ByteRange>,
    /// What occupies the destination, so a person can choose before
    /// completing.
    pub conflict: Option<Entry>,
}

/// One frame of an upload, client to server.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum UploadFrame {
    /// Bytes at `offset` in the file. Any order: a resumed upload sends
    /// only what is still needed.
    Chunk { offset: u64, bytes: Vec<u8> },
    /// No more frames on this call.
    Finished,
}

/// What a run of frames did.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Received {
    pub upload_id: UploadId,
    /// Bytes written by this run.
    pub written: u64,
    /// Still outstanding afterwards.
    pub needed: Vec<ByteRange>,
}

/// How far an upload has got: what a client that lost its connection
/// asks before sending the rest.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct UploadProgress {
    pub upload_id: UploadId,
    pub root_id: RootId,
    pub path: RootPath,
    pub received: u64,
    pub total: u64,
    pub needed: Vec<ByteRange>,
}

/// What to read: a file's current bytes or a past version's, whole or a
/// range of them.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ReadRequest {
    pub root_id: RootId,
    pub path: RootPath,
    /// A past version (needs `History`); `None` is the current one.
    pub version: Option<VersionId>,
    /// `None` reads it all. Clamped to the file.
    pub range: Option<ByteRange>,
}

/// A read's result, after its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ReadDone {
    pub content: ContentId,
    /// The whole file's length.
    pub total: u64,
    /// What was sent: the range, clamped.
    pub range: ByteRange,
}

/// Everything that happens to an org's files, on one subscription. A
/// subscriber hears only what it may see.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum FilesEvent {
    /// First, once: the subscription is live. Read current state after
    /// this, then fold what follows in.
    Attached,
    RootCreated(Root),
    RootRenamed(Root),
    RootReleased(RootId),
    /// Entries made or changed (a save, a move's destination, a folder).
    Changed {
        root_id: RootId,
        entries: Vec<Entry>,
    },
    /// Paths gone (a delete, a move's source).
    Removed {
        root_id: RootId,
        paths: Vec<RootPath>,
    },
    Labelled(VersionLabel),
    Unlabelled(VersionLabel),
    Granted(Grant),
    Revoked(Grant),
}
