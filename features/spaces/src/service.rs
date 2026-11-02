//! The wire (feature `vox`): one service, for the people who make and
//! manage spaces.
//!
//! The org is never an argument: a call runs in the org of the lane it
//! arrived on (the lane's scope: an `EngineHost` `Lane`, or
//! `architect_orgs::server::confine`), and the caller is the principal
//! the permissions gate resolved. Who may do what to a space is
//! the space's: its managers (`admin` on `spaces/<id>`) and the org's
//! owners and admins change it, its members see it.

use architect_permissions::{MethodPermit, ServicePermits};

use crate::model::{
    Member, NewSpace, Space, SpaceAudit, SpaceGrant, SpaceId, SpacePatch, SpacePolicy, SpaceRole,
    SpaceSummary, SpacesFault, Subject,
};

#[architect::rpc]
pub trait SpacesService {
    /// Make a space in the lane's org; the caller manages it. Needs a
    /// role in the org (below admin, only if the org's policy lets
    /// members create).
    async fn create_space(&self, new: NewSpace) -> Result<Space, SpacesFault>;

    /// The spaces the caller is in; an owner or admin of the org sees
    /// every space in it.
    async fn spaces(&self) -> Result<Vec<SpaceSummary>, SpacesFault>;

    /// One space, its roles and members: for its members and managers.
    async fn space(&self, id: SpaceId) -> Result<Space, SpacesFault>;

    /// Rename it, or change what it covers. A manager's call; what it no
    /// longer covers its members lose on their next call.
    async fn update_space(&self, id: SpaceId, patch: SpacePatch) -> Result<Space, SpacesFault>;

    /// Delete it: every membership in it ends on the next call.
    async fn delete_space(&self, id: SpaceId) -> Result<(), SpacesFault>;

    /// Add `subject` with `role`, or change their role (`None`: only their
    /// grants). A manager's call. `expires_us` ends the membership then.
    async fn set_member(
        &self,
        id: SpaceId,
        subject: Subject,
        role: Option<String>,
        expires_us: Option<i64>,
    ) -> Result<Member, SpacesFault>;

    /// Remove `subject`: binds on their next call.
    async fn remove_member(&self, id: SpaceId, subject: Subject) -> Result<(), SpacesFault>;

    /// Give `subject` actions on one resource inside the space, making
    /// them a member (with no role) if they are not one. Replaces an
    /// earlier grant on the same resource.
    async fn grant(
        &self,
        id: SpaceId,
        subject: Subject,
        grant: SpaceGrant,
    ) -> Result<Member, SpacesFault>;

    /// Withdraw `subject`'s grant on `resource`. A member left with no
    /// role and no grant is removed.
    async fn revoke_grant(
        &self,
        id: SpaceId,
        subject: Subject,
        resource: String,
    ) -> Result<Member, SpacesFault>;

    /// Define a role of the space's own (or replace a built-in of the
    /// same name, for this space).
    async fn set_role(&self, id: SpaceId, role: SpaceRole) -> Result<Space, SpacesFault>;

    /// Remove a role of the space's own. Refused while a member holds it.
    async fn remove_role(&self, id: SpaceId, name: String) -> Result<Space, SpacesFault>;

    /// What the caller holds in the space, as `(resource, actions)`
    /// rules after the org's policy: what a UI greys out by.
    async fn effective(&self, id: SpaceId) -> Result<Vec<SpaceGrant>, SpacesFault>;

    /// Every change to the space, oldest first. A manager's call.
    async fn audit(&self, id: SpaceId) -> Result<Vec<SpaceAudit>, SpacesFault>;

    /// The org's policy over its spaces: for anyone in the org.
    async fn policy(&self) -> Result<SpacePolicy, SpacesFault>;

    /// Change the org's policy. An owner's or admin's call; it binds on
    /// every member's next call.
    async fn set_policy(&self, policy: SpacePolicy) -> Result<SpacePolicy, SpacesFault>;
}

/// The gate's table for [`SpacesService`].
///
/// Every method is `read` on `spaces/**`, which the org's members hold
/// through the org's engine and a space's members through
/// `SpacesEngine` (every member reads `spaces/<id>`); the service decides
/// the rest per space.
pub const PERMITS: ServicePermits = ServicePermits {
    service: "spaces",
    methods: &[
        MethodPermit::new("create_space", "read", "spaces/**"),
        MethodPermit::new("spaces", "read", "spaces/**"),
        MethodPermit::new("space", "read", "spaces/**"),
        MethodPermit::new("update_space", "read", "spaces/**"),
        MethodPermit::new("delete_space", "read", "spaces/**").audited(),
        MethodPermit::new("set_member", "read", "spaces/**").audited(),
        MethodPermit::new("remove_member", "read", "spaces/**").audited(),
        MethodPermit::new("grant", "read", "spaces/**").audited(),
        MethodPermit::new("revoke_grant", "read", "spaces/**").audited(),
        MethodPermit::new("set_role", "read", "spaces/**"),
        MethodPermit::new("remove_role", "read", "spaces/**"),
        MethodPermit::new("effective", "read", "spaces/**"),
        MethodPermit::new("audit", "read", "spaces/**"),
        MethodPermit::new("policy", "read", "spaces/**"),
        MethodPermit::new("set_policy", "read", "spaces/**").audited(),
    ],
};
