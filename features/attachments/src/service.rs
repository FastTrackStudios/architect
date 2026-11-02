//! The wire (feature `vox`).
//!
//! The org is never an argument: a call runs in the org of the lane it
//! arrived on (`architect_orgs::server::confine`), and the caller is the
//! principal the permissions gate resolved. Every call names the parent,
//! and the parent decides: reading an attachment needs `read` on it (and
//! the parent must point at those bytes), attaching and uploading need
//! `write`.

use architect::vox::Tx;
use architect_files::{RenditionSet, Ticket};
use architect_permissions::{MethodPermit, ServicePermits};

use crate::model::{AttachFault, AttachUsage, Attachment, Parent, ReadDone, ReadRequest, Upload};

#[architect::rpc]
pub trait AttachmentsService {
    /// Begin (or resume) sending the bytes `hash` names for `parent`:
    /// how many the server has. All of them when the org's store holds
    /// them already, so the same bytes are sent once. Needs `write` on
    /// the parent, and room in the org's quota.
    async fn begin(&self, parent: Parent, hash: String, size: u64) -> Result<Upload, AttachFault>;

    /// Bytes of `hash` at `offset` (where the upload stands). When the
    /// last arrives they are checked against `hash` and stored, and their
    /// renditions asked for; bytes that hash otherwise are dropped and the
    /// upload starts again ([`AttachFault::Mismatch`]).
    async fn send(
        &self,
        parent: Parent,
        hash: String,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Upload, AttachFault>;

    /// Attach to a record (the server keeps its list) bytes the org
    /// holds; again replaces the details. A document lists its own
    /// ([`crate::doc::attach`]): this refuses one.
    async fn attach(
        &self,
        parent: Parent,
        attachment: Attachment,
    ) -> Result<Attachment, AttachFault>;

    /// Detach from a record; whether it was attached.
    async fn detach(&self, parent: Parent, hash: String) -> Result<bool, AttachFault>;

    /// The parent's attachments: a record's from the server's list, a
    /// document's from the server's copy of it.
    async fn list(&self, parent: Parent) -> Result<Vec<Attachment>, AttachFault>;

    /// The length of an attachment's bytes, or `None` while the server
    /// does not hold them (the device that attached them has not sent
    /// them yet).
    async fn size(&self, parent: Parent, hash: String) -> Result<Option<u64>, AttachFault>;

    /// An attachment's bytes (or a rendition's) from `offset` to the end,
    /// on `out`.
    async fn read(&self, request: ReadRequest, out: Tx<Vec<u8>>) -> Result<ReadDone, AttachFault>;

    /// An attachment's renditions (an image's poster, a video's proxy),
    /// asking for any that are missing.
    async fn renditions(&self, parent: Parent, hash: String) -> Result<RenditionSet, AttachFault>;

    /// A read ticket for the parent's attachments: what a browser appends
    /// to a byte URL (`crate::web`) for an `<img>` or `<video>`. Reads as
    /// the caller, and only while they may still read the parent.
    async fn ticket(&self, parent: Parent) -> Result<Ticket, AttachFault>;

    /// What the org's attachments hold, and may.
    async fn usage(&self) -> Result<AttachUsage, AttachFault>;
}

/// The gate's table for [`AttachmentsService`].
///
/// Every method is `read` on `**`: the gate's part is that the caller
/// holds something in the org (a member's role, or a space's grant — a
/// person from another org invited to one document), and the attachments
/// feature decides each call by its parent, through the same engine.
pub const PERMITS: ServicePermits = ServicePermits {
    service: "attachments",
    methods: &[
        MethodPermit::new("begin", "read", "**"),
        MethodPermit::new("send", "read", "**"),
        MethodPermit::new("attach", "read", "**"),
        MethodPermit::new("detach", "read", "**"),
        MethodPermit::new("list", "read", "**"),
        MethodPermit::new("size", "read", "**"),
        MethodPermit::new("read", "read", "**"),
        MethodPermit::new("renditions", "read", "**"),
        MethodPermit::new("ticket", "read", "**"),
        MethodPermit::new("usage", "read", "**"),
    ],
};
