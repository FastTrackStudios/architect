//! The wire for share links (feature `vox`): [`ShareLinks`], served by
//! the node that hosts the org, on its org lane (`/org/{slug}/vox`).
//!
//! The caller is the person the lane's gate resolved from their issuer
//! session; the vault is the node's, so the org's. Any member lists the
//! org's shares; making, changing and revoking one is a write, so a
//! member or above.

use architect_permissions::{MethodPermit, ServicePermits};

use crate::share::{NewShare, Share, ShareChange, ShareComment, ShareFault};

#[architect::rpc]
pub trait ShareLinks {
    /// Make a share link: a scope of one vault, who may open it, until
    /// when and what they may do. Its code is new and random.
    async fn create(&self, new: NewShare) -> Result<Share, ShareFault>;

    /// Every share of the node's vaults, revoked and expired ones too.
    async fn shares(&self) -> Result<Vec<Share>, ShareFault>;

    /// Change one: its audience, access, expiry, title or domain. The
    /// next request to the link is answered by the change.
    async fn update(&self, code: String, change: ShareChange) -> Result<Share, ShareFault>;

    /// Take it down: the link answers 404 from the next request on, and
    /// its built pages are removed. A revoke is final.
    async fn revoke(&self, code: String) -> Result<Share, ShareFault>;

    /// The comments left on a share, oldest first.
    async fn comments(&self, code: String) -> Result<Vec<ShareComment>, ShareFault>;
}

/// The gate's table for [`ShareLinks`].
pub const SHARE_PERMITS: ServicePermits = ServicePermits {
    service: "ShareLinks",
    methods: &[
        MethodPermit::new("create", "write", "vaults/**").audited(),
        MethodPermit::new("shares", "read", "vaults/**"),
        MethodPermit::new("update", "write", "vaults/**").audited(),
        MethodPermit::new("revoke", "write", "vaults/**").audited(),
        MethodPermit::new("comments", "read", "vaults/**"),
    ],
};
