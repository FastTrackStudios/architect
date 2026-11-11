//! The wire for share links (feature `vox`): [`ShareLinks`], served by
//! the node that hosts the org, on its org lane (`/org/{slug}/vox`).
//!
//! The caller is the person the lane's gate resolved from their issuer
//! session; the vault is the node's, so the org's. Any member lists the
//! org's shares; making, changing and revoking one is a write, so a
//! member or above.

use architect_permissions::{MethodPermit, ServicePermits};

use crate::share::{
    KeptLayers, Listen, NewAudience, NewShare, Share, ShareChange, ShareComment, ShareFault,
};

#[architect::rpc]
pub trait ShareLinks {
    /// Make a share link: a scope of one vault, who may open it, until
    /// when and what they may do. Its code is new and random.
    async fn create(&self, new: NewShare) -> Result<Share, ShareFault>;

    /// Every share of the node's vaults, revoked and expired ones too.
    async fn shares(&self) -> Result<Vec<Share>, ShareFault>;

    /// Change one: its audience, access, expiry, title, domain or how far
    /// it plays ([`ShareChange::Listen`]). The
    /// next request to the link is answered by the change.
    async fn update(&self, code: String, change: ShareChange) -> Result<Share, ShareFault>;

    /// Take it down: the link answers 404 from the next request on, and
    /// its built pages are removed. A revoke is final.
    async fn revoke(&self, code: String) -> Result<Share, ShareFault>;

    /// Add a named audience to share `code` (#510): the notes of its
    /// scope `new.filter` holds for (a Bases filter expression), on a
    /// link of its own, whose code is new and random. Answers the share,
    /// the audience in it.
    async fn add_audience(&self, code: String, new: NewAudience) -> Result<Share, ShareFault>;

    /// Remove the audience called `name` from share `code`: its link
    /// answers 404 from the next request on; the share's own link and
    /// its other audiences stay.
    async fn remove_audience(&self, code: String, name: String) -> Result<Share, ShareFault>;

    /// How far audience `name`'s link of share `code` plays (#528):
    /// `listen`, never past the share's; None: the share's.
    async fn audience_listen(
        &self,
        code: String,
        name: String,
        listen: Option<Listen>,
    ) -> Result<Share, ShareFault>;

    /// The layer rules the node keeps for extensions no longer installed
    /// (#528): they still place files, so removing an extension never
    /// opens a link.
    async fn kept_layers(&self) -> Result<Vec<KeptLayers>, ShareFault>;

    /// Forget the kept layer rules of `extension`: its files are placed by
    /// the rules installed now. Refused for an installed extension (its
    /// rules are its manifest's) and for one nothing is kept of.
    async fn forget_layers(&self, extension: String) -> Result<Vec<KeptLayers>, ShareFault>;

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
        MethodPermit::new("add_audience", "write", "vaults/**").audited(),
        MethodPermit::new("remove_audience", "write", "vaults/**").audited(),
        MethodPermit::new("audience_listen", "write", "vaults/**").audited(),
        MethodPermit::new("kept_layers", "read", "vaults/**"),
        MethodPermit::new("forget_layers", "write", "vaults/**").audited(),
        MethodPermit::new("comments", "read", "vaults/**"),
    ],
};
