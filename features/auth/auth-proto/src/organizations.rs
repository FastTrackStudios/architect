//! The organization surface: one `#[architect::rpc]` trait, both faces.
//!
//! Everything a relying party needs to answer "which orgs is this person
//! in, and what may they do there" — the orgs a session belongs to with
//! the caller's role in each, one org's members as people rather than
//! ids, and the writes that change either. The server mounts this over
//! vox and over HTTP from the same declaration; the clients are
//! generated with it.
//!
//! Every method takes the session `token` as its first argument. Over
//! HTTP that argument may ride `Authorization: Bearer …` instead of the
//! body — the generated router fills it in — so a browser keeps the
//! credential in a header exactly as a vox client keeps it in metadata.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{AuthFlowError, AuthInvitation, AuthMember, AuthOrganization, AuthUser};

/// An organization together with the caller's membership in it.
///
/// Deliberately one object rather than two lists to join client-side:
/// the pair `(slug, role)` is the whole answer to "may this principal
/// act here", and splitting it invites a caller to read one without the
/// other.
#[architect::wire]
#[derive(Eq)]
pub struct OrganizationBundle {
    pub organization: AuthOrganization,
    pub membership: AuthMember,
}

/// A membership with the person attached.
///
/// A member list that shows user ids is not a member list. The user is
/// resolved server-side so no caller has to fan out over the ids.
#[architect::wire]
#[derive(Eq)]
pub struct OrganizationMember {
    pub member: AuthMember,
    pub user: AuthUser,
}

/// A freshly issued invitation and the one-time token that redeems it.
///
/// The token is returned once, here, and never readable again — the
/// invitation row stores a hash. Whoever calls `invite_member` is
/// responsible for delivering it.
#[architect::wire]
#[derive(Eq)]
pub struct IssuedInvitation {
    pub invitation: AuthInvitation,
    pub token: String,
}

/// What a new organization is called. The caller becomes its owner.
#[architect::wire]
#[derive(Eq)]
pub struct NewOrganization {
    pub name: String,
    /// What relying parties key on: what an operator types, and what a
    /// deployment's own directories are named after. The id is
    /// meaningful only here.
    pub slug: String,
    pub logo: Option<String>,
    pub metadata_json: Option<String>,
}

/// Who to invite, and as what.
#[architect::wire]
#[derive(Eq)]
pub struct Invite {
    pub organization_id: Uuid,
    pub email: String,
    pub role: String,
    /// `None` means a week from now. An invitation with no expiry is a
    /// standing key, and making the caller name one every time is how it
    /// ends up pasted as a far-future constant. A week survives a holiday.
    pub expires_at: Option<DateTime<Utc>>,
}

// r[impl auth.transport.vox-schema]
//
// `path = "auth"` rather than the default `organization`: everything this
// server serves lives under `/auth`, and a relying party already told
// "the auth surface is /auth" should not have to learn that one group of
// it sits somewhere else. The method names already carry the noun
// (`list-organizations`, `create-organization`), so the extra segment
// bought nothing — and `path` takes a single segment by design, so
// `auth/organization` would have meant nesting the router by hand at the
// mount site to say the same thing.
#[architect::service(path = "auth")]
pub trait OrganizationService {
    /// Every org this session belongs to, with the caller's role in each.
    /// One call, one round trip, and a relying party needs no membership
    /// table of its own.
    #[http(path = "organization/list")]
    async fn list_organizations(
        &self,
        token: String,
    ) -> Result<Vec<OrganizationBundle>, AuthFlowError>;

    /// One org, if the caller is in it. A non-member gets the same answer
    /// as a missing org: membership is not something an outsider should
    /// be able to probe for.
    #[http(path = "organization/get")]
    async fn get_organization(
        &self,
        token: String,
        organization_id: Uuid,
    ) -> Result<OrganizationBundle, AuthFlowError>;

    /// Who is in an org — people, not ids.
    #[http(path = "organization/members")]
    async fn list_members(
        &self,
        token: String,
        organization_id: Uuid,
    ) -> Result<Vec<OrganizationMember>, AuthFlowError>;

    /// Create an org; the caller becomes its owner.
    #[http(path = "organization/create")]
    async fn create_organization(
        &self,
        token: String,
        organization: NewOrganization,
    ) -> Result<OrganizationBundle, AuthFlowError>;

    /// Make one of the caller's orgs the session's active one.
    #[http(path = "organization/set-active")]
    async fn set_active_organization(
        &self,
        token: String,
        organization_id: Uuid,
    ) -> Result<(), AuthFlowError>;

    /// Issue an invitation. The returned token is shown once.
    #[http(path = "organization/invite-member")]
    async fn invite_member(
        &self,
        token: String,
        invite: Invite,
    ) -> Result<IssuedInvitation, AuthFlowError>;

    /// Redeem an invitation as the signed-in caller, whoever was invited.
    #[http(path = "organization/accept-invitation")]
    async fn accept_invitation(
        &self,
        token: String,
        invitation_id: Uuid,
        invitation_token: String,
    ) -> Result<(), AuthFlowError>;

    /// Change a member's role.
    ///
    /// Promoting somebody to `owner` is half of handing an organization
    /// over; [`Self::leave_organization`] is the other half. Together
    /// they are a transfer: make them an owner, then stop being one.
    /// There is no single "transfer" verb, because two steps in this
    /// order mean the organization is never ownerless in between — and
    /// a verb that did both would have to invent an answer for the case
    /// where the second half fails.
    #[http(path = "organization/update-member-role")]
    async fn update_member_role(
        &self,
        token: String,
        organization_id: Uuid,
        user_id: Uuid,
        role: String,
    ) -> Result<AuthMember, AuthFlowError>;

    /// Remove somebody else. Needs `member:delete`.
    #[http(path = "organization/remove-member")]
    async fn remove_member(
        &self,
        token: String,
        organization_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), AuthFlowError>;

    /// Stop being a member yourself.
    ///
    /// Deliberately not the same call as [`Self::remove_member`]:
    /// removing somebody else needs permission over them, and nobody
    /// needs permission to leave a room. The last-owner rule applies to
    /// both, so an organization cannot be left ownerless — the final
    /// owner has to promote a successor before stepping out, which is
    /// exactly what makes the pair above a safe transfer.
    #[http(path = "organization/leave")]
    async fn leave_organization(
        &self,
        token: String,
        organization_id: Uuid,
    ) -> Result<(), AuthFlowError>;
}
