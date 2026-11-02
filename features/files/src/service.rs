//! The wire (feature `vox`): one service, its events beside it.
//!
//! The org is never an argument: a call runs in the org of the lane it
//! arrived on (the lane's scope: an `EngineHost` `Lane`, or
//! `architect_orgs::server::confine`), and the caller is the principal
//! the permissions gate resolved. Bytes ride vox channels — an
//! upload's frames in ([`FilesService::send_bytes`]), a read's out
//! ([`FilesService::read`]) — with vox's credit as flow control, so
//! neither side buffers a whole file.

use architect::vox::{Rx, Tx};
use architect_permissions::{MethodPermit, ServicePermits};

use crate::access::{Capability, Effective, Grant};
use crate::error::FilesFault;
use crate::id::{GrantId, LabelId, RootId, UploadId, VersionId};
use crate::model::{
    Entry, FilesEvent, OnConflict, ReadDone, ReadRequest, Received, Relocation, Root, UploadFrame,
    UploadPlan, UploadProgress, UploadSpec, Usage, Version, VersionLabel, WriteReceipt,
};
use crate::path::RootPath;
use crate::rendition::{RenditionSet, Ticket};

#[architect::rpc]
pub trait FilesService {
    // ── Roots ──────────────────────────────────────────────────────

    /// The root at `dir` in the org's files, made (named `name`) if there
    /// is none. Idempotent on `dir`, so an app calls it on every start to
    /// find its store. Making one needs `Write` in the org.
    async fn ensure_root(&self, dir: String, name: String) -> Result<Root, FilesFault>;

    /// The roots the caller can see: every root for a member, the roots
    /// they hold a grant in otherwise.
    async fn roots(&self) -> Result<Vec<Root>, FilesFault>;

    async fn root(&self, root_id: RootId) -> Result<Root, FilesFault>;

    /// Rename a root (its display name). An owner's or admin's call.
    async fn rename_root(&self, root_id: RootId, name: String) -> Result<Root, FilesFault>;

    /// Forget a root: its tree, history, labels and grants. An owner's or
    /// admin's call. The bytes stay in the blob store (another root, or a
    /// document, may hold them).
    async fn release_root(&self, root_id: RootId) -> Result<(), FilesFault>;

    /// The org's space and quota.
    async fn usage(&self) -> Result<Usage, FilesFault>;

    // ── The tree ───────────────────────────────────────────────────

    /// A folder's children the caller can see.
    async fn browse(&self, root_id: RootId, path: RootPath) -> Result<Vec<Entry>, FilesFault>;

    /// One file or folder.
    async fn entry(&self, root_id: RootId, path: RootPath) -> Result<Entry, FilesFault>;

    /// Make folders, and any missing above them. Existing ones are no
    /// error, so a retry after a dropped connection succeeds.
    async fn create_dirs(
        &self,
        root_id: RootId,
        paths: Vec<RootPath>,
    ) -> Result<WriteReceipt, FilesFault>;

    /// Rename in place (same folder). A file keeps its history.
    async fn rename(
        &self,
        root_id: RootId,
        path: RootPath,
        name: String,
    ) -> Result<WriteReceipt, FilesFault>;

    /// Move a set of paths, all or none. Files keep their history.
    async fn move_paths(
        &self,
        root_id: RootId,
        moves: Vec<Relocation>,
        on_conflict: OnConflict,
    ) -> Result<WriteReceipt, FilesFault>;

    /// Copy a set of paths, all or none. No bytes move: the copy is the
    /// same content.
    async fn copy_paths(
        &self,
        root_id: RootId,
        copies: Vec<Relocation>,
        on_conflict: OnConflict,
    ) -> Result<WriteReceipt, FilesFault>;

    /// Delete a set of paths, all or none. A deleted file's history
    /// stays: [`Self::restore`] brings it back.
    async fn delete_paths(
        &self,
        root_id: RootId,
        paths: Vec<RootPath>,
    ) -> Result<WriteReceipt, FilesFault>;

    // ── Bytes ──────────────────────────────────────────────────────

    /// Ask what to send. `needed` is empty when the org already holds
    /// these bytes: complete at once.
    async fn begin_upload(&self, spec: UploadSpec) -> Result<UploadPlan, FilesFault>;

    /// What an upload still needs: where a client resumes.
    async fn upload_progress(&self, upload_id: UploadId) -> Result<UploadProgress, FilesFault>;

    /// Send an upload's bytes: the client keeps the sender, passes the
    /// receiver. Lands nothing — that is [`Self::complete_upload`].
    async fn send_bytes(
        &self,
        upload_id: UploadId,
        frames: Rx<UploadFrame>,
    ) -> Result<Received, FilesFault>;

    /// Land the upload, choosing what to do if the destination is taken.
    async fn complete_upload(
        &self,
        upload_id: UploadId,
        on_conflict: OnConflict,
    ) -> Result<Entry, FilesFault>;

    /// Abandon it. Nothing partial was ever in the tree.
    async fn abort_upload(&self, upload_id: UploadId) -> Result<(), FilesFault>;

    /// Read a file's bytes (or a range of them, or a past version's)
    /// into `out`.
    async fn read(&self, request: ReadRequest, out: Tx<Vec<u8>>) -> Result<ReadDone, FilesFault>;

    /// A read ticket for `path` and everything under it: what a browser
    /// appends to a byte URL (`crate::web`) for an element that cannot
    /// send a header. Needs `Read` there; reads as the caller, and only
    /// what they may still read when it is used.
    async fn ticket(&self, root_id: RootId, path: RootPath) -> Result<Ticket, FilesFault>;

    // ── Renditions ─────────────────────────────────────────────────

    /// A version's renditions (`None`: the current one), asking for any
    /// that are missing. Needs `Read` (and `History` for a past version).
    async fn renditions(
        &self,
        root_id: RootId,
        path: RootPath,
        version: Option<VersionId>,
    ) -> Result<RenditionSet, FilesFault>;

    // ── Versions ───────────────────────────────────────────────────

    /// A file's versions, oldest first (deletes included).
    async fn history(&self, root_id: RootId, path: RootPath) -> Result<Vec<Version>, FilesFault>;

    /// Make a past version the current one: a new version with its
    /// bytes. Brings back a deleted file.
    async fn restore(
        &self,
        root_id: RootId,
        path: RootPath,
        version: VersionId,
    ) -> Result<Entry, FilesFault>;

    /// Name a version of a file. A name is unique per file; naming
    /// another version with it is [`FilesFault::Exists`].
    async fn label(
        &self,
        root_id: RootId,
        path: RootPath,
        version: VersionId,
        name: String,
    ) -> Result<VersionLabel, FilesFault>;

    async fn unlabel(&self, label_id: LabelId) -> Result<VersionLabel, FilesFault>;

    /// A file's labels, or (no path) every label in the root the caller
    /// can see.
    async fn labels(
        &self,
        root_id: RootId,
        path: Option<RootPath>,
    ) -> Result<Vec<VersionLabel>, FilesFault>;

    // ── Access ─────────────────────────────────────────────────────

    /// Give `user` capabilities over a folder. Needs `Share` there, and
    /// conveys no more than the granter holds there.
    async fn grant(
        &self,
        root_id: RootId,
        path: RootPath,
        user: String,
        capabilities: Vec<Capability>,
    ) -> Result<Grant, FilesFault>;

    /// Withdraw a grant; binds on the grantee's next call.
    async fn revoke(&self, grant_id: GrantId) -> Result<Grant, FilesFault>;

    /// The grants governing a path (or a whole root). Needs `Share`.
    async fn grants(
        &self,
        root_id: RootId,
        path: Option<RootPath>,
    ) -> Result<Vec<Grant>, FilesFault>;

    /// What the caller may do at a path.
    async fn effective(&self, root_id: RootId, path: RootPath) -> Result<Effective, FilesFault>;

    /// Every change the caller may see, after a first
    /// [`FilesEvent::Attached`]; `Some(root)` narrows to one root.
    #[subscribe]
    fn events(&self, root_id: Option<RootId>) -> FilesEvent;
}

/// The gate's table for [`FilesService`].
///
/// Every method is `read` on `files/**`, so the gate's part is membership
/// (a `ConfinedEngine` refuses a non-member and another org's lane) and
/// the files feature decides the rest per path, by role and grant — a
/// viewer granted `Write` on one folder must reach `complete_upload` to
/// use it.
pub const PERMITS: ServicePermits = ServicePermits {
    service: "files",
    methods: &[
        MethodPermit::new("ensure_root", "read", "files/**"),
        MethodPermit::new("roots", "read", "files/**"),
        MethodPermit::new("root", "read", "files/**"),
        MethodPermit::new("rename_root", "read", "files/**"),
        MethodPermit::new("release_root", "read", "files/**"),
        MethodPermit::new("usage", "read", "files/**"),
        MethodPermit::new("browse", "read", "files/**"),
        MethodPermit::new("entry", "read", "files/**"),
        MethodPermit::new("create_dirs", "read", "files/**"),
        MethodPermit::new("rename", "read", "files/**"),
        MethodPermit::new("move_paths", "read", "files/**"),
        MethodPermit::new("copy_paths", "read", "files/**"),
        MethodPermit::new("delete_paths", "read", "files/**"),
        MethodPermit::new("begin_upload", "read", "files/**"),
        MethodPermit::new("upload_progress", "read", "files/**"),
        MethodPermit::new("send_bytes", "read", "files/**"),
        MethodPermit::new("complete_upload", "read", "files/**"),
        MethodPermit::new("abort_upload", "read", "files/**"),
        MethodPermit::new("read", "read", "files/**"),
        MethodPermit::new("ticket", "read", "files/**"),
        MethodPermit::new("renditions", "read", "files/**"),
        MethodPermit::new("history", "read", "files/**"),
        MethodPermit::new("restore", "read", "files/**"),
        MethodPermit::new("label", "read", "files/**"),
        MethodPermit::new("unlabel", "read", "files/**"),
        MethodPermit::new("labels", "read", "files/**"),
        MethodPermit::new("grant", "read", "files/**"),
        MethodPermit::new("revoke", "read", "files/**"),
        MethodPermit::new("grants", "read", "files/**"),
        MethodPermit::new("effective", "read", "files/**"),
    ],
};

/// The gate's table for the events stream ([`FilesServiceStreamClient`]).
pub const STREAM_PERMITS: ServicePermits = ServicePermits {
    service: "files-stream",
    methods: &[MethodPermit::new("events", "read", "files/**")],
};
