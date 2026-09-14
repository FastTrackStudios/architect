//! Organizations: the list, one organization, and the two ways in.
//!
//! # Two doors, deliberately
//!
//! An *invitation* names an address and is accepted once — the right
//! shape for "join us, Ada". A *link* names nobody, is pasted into a
//! channel, and admits everyone who follows it — the right shape for a
//! cohort. They are separate on this page because they fail
//! differently: a stale invitation is one person waiting, and a leaked
//! link is everybody who has it. That is why a link shows its uses and
//! carries a revoke button, and an invitation does not.
//!
//! # What the token is
//!
//! Both doors are opened by a secret in the URL, and both preview
//! pages are unauthenticated. That is the point: somebody following an
//! invitation has no session yet, and being asked to sign up before
//! being told what for is how invitations go unaccepted. The preview
//! reveals nothing that the holder of the link does not already have.

use architect_auth::{
    AcceptInvitation, AddTeamMember, AuthStorage, CancelInvitation, ClaimInvitation,
    CreateInvitation, CreateInviteLink, CreateOrganization, CreateTeam, CurrentSession,
    DeleteOrganization, DeleteTeam, LeaveOrganization, LinkAgent, ListAgents, ListInvitations,
    ListInviteLinks, ListMembers, ListMyInvitations, ListOrganizations, ListTeamMembers, ListTeams,
    PreviewInvitation, PreviewInviteLink, RedeemInviteLink, RejectInvitation, RemoveMember,
    RemoveTeamMember, RevokeInviteLink, SetMemberRole, UnlinkAgent, UpdateOrganization,
};
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Redirect, Response};
use chrono::Utc;
use dioxus::prelude::*;
use uuid::Uuid;

use crate::UiState;
use crate::page::{Flash, document, flash_to, sign_in_first, token_of};
use crate::profile::message;
use crate::settings::Nav;
use crate::views::{
    AgentRow, DeadEnd, Declined, InvitationRow, InvitationView, JoinView, LinkRow, MemberRow,
    MyInvitationRow, OrgRow, OrgView, OrgsView, TeamRow,
};

/// How long an emailed invitation stays good for.
///
/// Long enough to survive a holiday, short enough that a forwarded
/// mailbox is not a standing key.
const INVITATION_DAYS: i64 = 14;

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Set once, after minting a link: the only time its token is
    /// legible, so it is handed back through the redirect rather than
    /// stored anywhere.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateOrgForm {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateOrgForm {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub logo: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct MemberForm {
    pub user_id: Uuid,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct TeamForm {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub team_id: Option<Uuid>,
    #[serde(default)]
    pub user_id: Option<Uuid>,
}

#[derive(Debug, serde::Deserialize)]
pub struct InviteForm {
    pub email: String,
    pub role: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct IdForm {
    pub id: Uuid,
}

#[derive(Debug, serde::Deserialize)]
pub struct LinkForm {
    pub role: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub max_uses: String,
    #[serde(default)]
    pub expires_days: String,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct TokenQuery {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TokenForm {
    /// Absent when the form came from the account page rather than
    /// from a mailed link. The link path proves entitlement with this
    /// token; the account path proves it with the signed-in address,
    /// so there is nothing to carry.
    #[serde(default)]
    pub token: String,
}

// ── The list ─────────────────────────────────────────────────────────

/// `GET /orgs`
pub async fn index<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first("/orgs");
    };
    let Ok(bundles) = state
        .auth
        .list_organizations(ListOrganizations {
            session_token: token,
        })
        .await
    else {
        return sign_in_first("/orgs");
    };
    let rows: Vec<OrgRow> = bundles
        .into_iter()
        .map(|bundle| OrgRow {
            id: bundle.organization.id,
            name: bundle.organization.name,
            slug: bundle.organization.slug,
            role: bundle.membership.role,
        })
        .collect();
    let Some(nav) = Nav::build(&state, &headers, "/orgs").await else {
        return sign_in_first("/orgs");
    };
    // What is waiting for a decision. Best effort: an invitation list
    // that cannot be read is no reason to refuse the page somebody came
    // here to see, so a failure shows no invitations rather than no
    // organizations.
    let invitations = pending_invitations(&state, &headers).await;
    let agents = linked_agents(&state, &headers).await;
    crate::settings::document(
        "Organizations",
        "Organizations",
        "The organizations this account belongs to.",
        &nav,
        rsx! {
            OrgsView {
                rows,
                invitations,
                agents,
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// The agents this account has linked. Best effort, like the
/// invitations: a list that cannot be read hides the panel's rows, not
/// the page.
async fn linked_agents<S>(state: &UiState<S>, headers: &HeaderMap) -> Vec<AgentRow>
where
    S: AuthStorage,
{
    let Some(token) = token_of(headers, &state.cookie) else {
        return Vec::new();
    };
    let Ok(agents) = state
        .auth
        .list_agents(ListAgents {
            session_token: token,
        })
        .await
    else {
        return Vec::new();
    };
    agents
        .into_iter()
        .map(|agent| AgentRow {
            id: agent.link.id,
            who: agent
                .agent_email
                .or(agent.agent_name)
                .unwrap_or_else(|| agent.link.agent_user_id.to_string()),
            cap: agent.link.max_role,
        })
        .collect()
}

#[derive(Debug, serde::Deserialize)]
pub struct LinkAgentForm {
    #[serde(default)]
    pub agent_email: String,
    #[serde(default)]
    pub max_role: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct UnlinkAgentForm {
    pub link_id: Uuid,
}

/// `POST /account/agents/link`
pub async fn link_agent<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<LinkAgentForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first("/orgs");
    };
    match state
        .auth
        .link_agent(LinkAgent {
            session_token: token,
            agent_email: form.agent_email,
            max_role: form.max_role,
        })
        .await
    {
        Ok(_) => flash_to("/orgs", &Flash::Ok("Agent linked.".into())),
        Err(error) => flash_to("/orgs", &Flash::Error(message(&error))),
    }
}

/// `POST /account/agents/unlink`
pub async fn unlink_agent<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<UnlinkAgentForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first("/orgs");
    };
    match state
        .auth
        .unlink_agent(UnlinkAgent {
            session_token: token,
            link_id: form.link_id,
        })
        .await
    {
        Ok(()) => flash_to("/orgs", &Flash::Ok("Agent removed.".into())),
        Err(error) => flash_to("/orgs", &Flash::Error(message(&error))),
    }
}

/// Invitations addressed to the signed-in person, named where possible.
///
/// The organization's name needs a read the invitee is not entitled to
/// — they are not a member yet — so it is looked up directly rather
/// than through `get_organization`. Asking somebody to accept something
/// identified only by a uuid is not an invitation; the id is the
/// fallback, not the plan.
async fn pending_invitations<S>(state: &UiState<S>, headers: &HeaderMap) -> Vec<MyInvitationRow>
where
    S: AuthStorage,
{
    let Some(token) = token_of(headers, &state.cookie) else {
        return Vec::new();
    };
    let Ok(invitations) = state
        .auth
        .list_my_invitations(ListMyInvitations {
            session_token: token,
        })
        .await
    else {
        return Vec::new();
    };
    let mut rows = Vec::with_capacity(invitations.len());
    for invitation in invitations {
        let organization = state
            .auth
            .storage
            .find_organization_by_id(invitation.organization_id)
            .await
            .ok()
            .flatten()
            .map_or_else(|| invitation.organization_id.to_string(), |org| org.name);
        rows.push(MyInvitationRow {
            id: invitation.id,
            organization,
            role: invitation.role,
        });
    }
    rows
}

/// `POST /orgs`
pub async fn create<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CreateOrgForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first("/orgs");
    };
    // An empty slug is derived from the name rather than rejected:
    // being made to invent a URL fragment before you can make a
    // workspace is friction for nothing.
    let slug = if form.slug.trim().is_empty() {
        slugify(&form.name)
    } else {
        form.slug.trim().to_owned()
    };
    match state
        .auth
        .create_organization(CreateOrganization {
            session_token: token,
            name: form.name.trim().to_owned(),
            slug,
            logo: None,
            metadata_json: None,
        })
        .await
    {
        Ok(bundle) => Redirect::to(&format!("/orgs/{}", bundle.organization.id)).into_response(),
        Err(error) => flash_to("/orgs", &Flash::Error(message(&error))),
    }
}

/// A name reduced to a URL-safe slug.
///
/// Runs of anything that is not a letter or digit become one dash, and
/// the ends are trimmed, so "Acme Records, Inc." is `acme-records-inc`
/// rather than `acme-records--inc-`.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

// ── One organization ─────────────────────────────────────────────────

/// `GET /orgs/{id}`
pub async fn show<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<PageQuery>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    let Ok(bundle) = state
        .auth
        .get_organization(architect_auth::GetOrganization {
            session_token: token.clone(),
            organization_id: id,
        })
        .await
    else {
        return flash_to(
            "/orgs",
            &Flash::Error("That organization is not available to you.".into()),
        );
    };

    let members = state
        .auth
        .list_members(ListMembers {
            session_token: token.clone(),
            organization_id: id,
        })
        .await
        .unwrap_or_default();
    let teams = state
        .auth
        .list_teams(ListTeams {
            session_token: token.clone(),
            organization_id: id,
        })
        .await
        .unwrap_or_default();
    let mut team_rows = Vec::with_capacity(teams.len());
    for team in teams {
        let member_ids = state
            .auth
            .list_team_members(ListTeamMembers {
                session_token: token.clone(),
                organization_id: id,
                team_id: team.id,
            })
            .await
            .unwrap_or_default();
        team_rows.push(TeamRow {
            id: team.id,
            name: team.name,
            members: member_ids
                .iter()
                .filter_map(|team_member| {
                    members
                        .iter()
                        .find(|m| m.member.user_id == team_member.user_id)
                        .map(|m| display_name(m))
                })
                .collect(),
        });
    }

    // Only somebody who may invite is shown the invitation machinery.
    // These calls carry the permission check, so an empty result and a
    // refusal are the same thing here: nothing to render.
    let invitations = state
        .auth
        .list_invitations(ListInvitations {
            session_token: token.clone(),
            organization_id: id,
        })
        .await
        .unwrap_or_default();
    let links = state
        .auth
        .list_invite_links(ListInviteLinks {
            session_token: token.clone(),
            organization_id: id,
        })
        .await
        .unwrap_or_default();

    let now = Utc::now();
    let can_invite = matches!(bundle.membership.role.as_str(), "owner" | "admin");
    let Some(nav) = Nav::build(&state, &headers, &path).await else {
        return sign_in_first(&path);
    };
    let heading = bundle.organization.name.clone();
    let blurb = format!(
        "/{} · you are {}",
        bundle.organization.slug, bundle.membership.role
    );
    crate::settings::document(
        &heading,
        &heading,
        &blurb,
        &nav,
        rsx! {
            OrgView {
                id,
                name: bundle.organization.name,
                slug: bundle.organization.slug,
                logo: bundle.organization.logo.unwrap_or_default(),
                my_role: bundle.membership.role.clone(),
                is_owner: bundle.membership.role == "owner",
                can_invite,
                members: members
                    .iter()
                    .map(|m| MemberRow {
                        user_id: m.member.user_id,
                        name: display_name(m),
                        email: m.user.email.clone().unwrap_or_default(),
                        role: m.member.role.clone(),
                    })
                    .collect::<Vec<_>>(),
                teams: team_rows,
                invitations: invitations
                    .into_iter()
                    .map(|invitation| InvitationRow {
                        id: invitation.id,
                        email: invitation.email,
                        role: invitation.role,
                        status: invitation.status,
                        expires: invitation.expires_at.format("%Y-%m-%d").to_string(),
                    })
                    .collect::<Vec<_>>(),
                links: links
                    .into_iter()
                    .map(|link| LinkRow {
                        used: link.uses,
                        remaining: link
                            .uses_remaining()
                            .map_or_else(|| "unlimited".to_owned(), |n| n.to_string()),
                        usable: link.is_usable_at(now),
                        id: link.id,
                        label: link.label.unwrap_or_else(|| "Invite link".to_owned()),
                        role: link.role,
                    })
                    .collect::<Vec<_>>(),
                minted: q.token,
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// The best name we have for somebody, never an empty cell.
fn display_name(member: &architect_auth::OrganizationMember) -> String {
    member
        .user
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| member.user.username.clone())
        .or_else(|| member.user.email.clone())
        .unwrap_or_else(|| member.member.user_id.to_string())
}

/// `POST /orgs/{id}` — rename, re-slug, set the logo.
pub async fn update<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<UpdateOrgForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    let logo = form.logo.trim();
    match state
        .auth
        .update_organization(UpdateOrganization {
            session_token: token,
            organization_id: id,
            name: Some(form.name.trim().to_owned()),
            slug: Some(form.slug.trim().to_owned()),
            // An empty box clears the logo rather than leaving the old
            // one in place, which is what a person emptying it means.
            logo: Some((!logo.is_empty()).then(|| logo.to_owned())),
            metadata_json: None,
        })
        .await
    {
        Ok(_) => flash_to(&path, &Flash::Ok("Organization saved.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/delete`
pub async fn delete<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .delete_organization(DeleteOrganization {
            session_token: token,
            organization_id: id,
        })
        .await
    {
        Ok(()) => flash_to("/orgs", &Flash::Ok("Organization deleted.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/leave`
pub async fn leave<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .leave_organization(LeaveOrganization {
            session_token: token,
            organization_id: id,
        })
        .await
    {
        Ok(()) => flash_to(
            "/orgs",
            &Flash::Ok("You have left the organization.".into()),
        ),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/members/role`
pub async fn set_role<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<MemberForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .set_member_role(SetMemberRole {
            session_token: token,
            organization_id: id,
            user_id: form.user_id,
            role: form.role,
        })
        .await
    {
        Ok(_) => flash_to(&path, &Flash::Ok("Role updated.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/members/remove`
pub async fn remove_member<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<MemberForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .remove_member(RemoveMember {
            session_token: token,
            organization_id: id,
            user_id: form.user_id,
        })
        .await
    {
        Ok(()) => flash_to(&path, &Flash::Ok("Member removed.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

// ── Teams ────────────────────────────────────────────────────────────

/// `POST /orgs/{id}/teams`
pub async fn create_team<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<TeamForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .create_team(CreateTeam {
            session_token: token,
            organization_id: id,
            name: form.name.trim().to_owned(),
        })
        .await
    {
        Ok(_) => flash_to(&path, &Flash::Ok("Team created.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/teams/delete`
pub async fn delete_team<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<TeamForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    let Some(team_id) = form.team_id else {
        return flash_to(&path, &Flash::Error("No team was named.".into()));
    };
    match state
        .auth
        .delete_team(DeleteTeam {
            session_token: token,
            organization_id: id,
            team_id,
        })
        .await
    {
        Ok(()) => flash_to(&path, &Flash::Ok("Team deleted.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/teams/members`
pub async fn add_team_member<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<TeamForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    let (Some(team_id), Some(user_id)) = (form.team_id, form.user_id) else {
        return flash_to(&path, &Flash::Error("Pick a team and a person.".into()));
    };
    match state
        .auth
        .add_team_member(AddTeamMember {
            session_token: token,
            organization_id: id,
            team_id,
            user_id,
        })
        .await
    {
        Ok(_) => flash_to(&path, &Flash::Ok("Added to the team.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/teams/members/remove`
pub async fn remove_team_member<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<TeamForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    let (Some(team_id), Some(user_id)) = (form.team_id, form.user_id) else {
        return flash_to(&path, &Flash::Error("Pick a team and a person.".into()));
    };
    match state
        .auth
        .remove_team_member(RemoveTeamMember {
            session_token: token,
            organization_id: id,
            team_id,
            user_id,
        })
        .await
    {
        Ok(()) => flash_to(&path, &Flash::Ok("Removed from the team.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

// ── Invitations ──────────────────────────────────────────────────────

/// `POST /orgs/{id}/invitations`
pub async fn invite<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<InviteForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .create_invitation(CreateInvitation {
            session_token: token,
            organization_id: id,
            email: form.email.trim().to_owned(),
            role: form.role,
            expires_at: architect_auth::expiry::in_days(INVITATION_DAYS),
        })
        .await
    {
        // The token goes back in the redirect so the page can show the
        // link to copy. It is not stored anywhere legible, so this is
        // the only chance to show it.
        Ok(invitation) => flash_to(
            &format!(
                "{path}?token={}",
                architect_auth::percent::encode_component(&format!(
                    "/invite/{}?token={}",
                    invitation.invitation.id, invitation.token
                ))
            ),
            &Flash::Ok("Invitation created.".into()),
        ),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/invitations/cancel`
pub async fn cancel_invitation<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<IdForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .cancel_invitation(CancelInvitation {
            session_token: token,
            invitation_id: form.id,
        })
        .await
    {
        Ok(()) => flash_to(&path, &Flash::Ok("Invitation cancelled.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

// ── Invite links ─────────────────────────────────────────────────────

/// `POST /orgs/{id}/links`
pub async fn create_link<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<LinkForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    // Blank means "no limit" for both, which is why they are parsed
    // leniently: a stray space should not become "0 uses".
    let max_uses = form.max_uses.trim().parse::<i32>().ok().filter(|n| *n > 0);
    let expires_at = form
        .expires_days
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .map(architect_auth::expiry::in_days);
    let label = form.label.trim();
    match state
        .auth
        .create_invite_link(CreateInviteLink {
            session_token: token,
            organization_id: id,
            role: form.role,
            label: (!label.is_empty()).then(|| label.to_owned()),
            expires_at,
            max_uses,
        })
        .await
    {
        Ok(minted) => flash_to(
            &format!(
                "{path}?token={}",
                architect_auth::percent::encode_component(&format!("/join?token={}", minted.token))
            ),
            &Flash::Ok("Invite link created.".into()),
        ),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

/// `POST /orgs/{id}/links/revoke`
pub async fn revoke_link<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<IdForm>,
) -> Response
where
    S: AuthStorage,
{
    let path = format!("/orgs/{id}");
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&path);
    };
    match state
        .auth
        .revoke_invite_link(RevokeInviteLink {
            session_token: token,
            link_id: form.id,
        })
        .await
    {
        Ok(()) => flash_to(&path, &Flash::Ok("Link revoked.".into())),
        Err(error) => flash_to(&path, &Flash::Error(message(&error))),
    }
}

// ── Coming in ────────────────────────────────────────────────────────

/// `GET /invite/{id}` — what you have been invited to.
pub async fn invitation_page<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<TokenQuery>,
) -> Response
where
    S: AuthStorage,
{
    let token = q.token.unwrap_or_default();
    let Ok(preview) = state
        .auth
        .preview_invitation(PreviewInvitation {
            invitation_id: id,
            token: token.clone(),
        })
        .await
    else {
        return document("Invitation", rsx! { DeadEnd {} });
    };
    let signed_in = signed_in(&state, &headers).await;
    document(
        "Invitation",
        rsx! {
            InvitationView {
                id,
                token,
                organization: preview.organization_name,
                role: preview.role,
                email: preview.email,
                signed_in,
                error: q.error,
            }
        },
    )
}

/// `POST /invite/{id}/accept`
pub async fn accept_invitation<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(form): Form<TokenForm>,
) -> Response
where
    S: AuthStorage,
{
    // No token means this came from the account page, where the form
    // has no link to carry one from — so there is no invitation page to
    // send a failure back to either.
    let from_link = !form.token.is_empty();
    let back = if from_link {
        format!(
            "/invite/{id}?token={}",
            architect_auth::percent::encode_component(&form.token)
        )
    } else {
        "/orgs".to_owned()
    };
    let Some(session_token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&back);
    };
    // Two proofs of the same entitlement. The link carries a token,
    // which is the only thing that distinguishes its holder; the account
    // page carries none, and is answered by matching the invitation's
    // address against the session's — which is the stricter of the two,
    // since the token path never checks the address at all.
    let outcome = if from_link {
        state
            .auth
            .accept_invitation(AcceptInvitation {
                session_token,
                invitation_id: id,
                token: form.token,
            })
            .await
    } else {
        state
            .auth
            .claim_invitation(ClaimInvitation {
                session_token,
                invitation_id: id,
            })
            .await
    };
    match outcome {
        Ok(()) => flash_to("/orgs", &Flash::Ok("You have joined.".into())),
        Err(error) => flash_to(&back, &Flash::Error(message(&error))),
    }
}

/// `POST /invite/{id}/reject`
pub async fn reject_invitation<S>(
    State(state): State<UiState<S>>,
    Path(id): Path<Uuid>,
    Form(form): Form<TokenForm>,
) -> Response
where
    S: AuthStorage,
{
    // No session needed: whoever holds the token is who the invitation
    // was for, and declining should not require signing up first.
    let _ = state
        .auth
        .reject_invitation(RejectInvitation {
            invitation_id: id,
            token: form.token,
        })
        .await;
    document("Invitation declined", rsx! { Declined {} })
}

/// `GET /join?token=…` — where an invite link lands.
pub async fn join_page<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Response
where
    S: AuthStorage,
{
    let token = q.token.unwrap_or_default();
    let Ok(preview) = state
        .auth
        .preview_invite_link(PreviewInviteLink {
            token: token.clone(),
        })
        .await
    else {
        return document("Invite link", rsx! { DeadEnd {} });
    };
    let signed_in = signed_in(&state, &headers).await;
    document(
        "Join {preview.organization_name}",
        rsx! {
            JoinView {
                token,
                organization: preview.organization_name,
                role: preview.role,
                signed_in,
                error: q.error,
            }
        },
    )
}

/// `POST /join`
pub async fn join<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response
where
    S: AuthStorage,
{
    let back = format!(
        "/join?token={}",
        architect_auth::percent::encode_component(&form.token)
    );
    let Some(session_token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&back);
    };
    match state
        .auth
        .redeem_invite_link(RedeemInviteLink {
            session_token,
            token: form.token,
        })
        .await
    {
        Ok(member) => flash_to(
            &format!("/orgs/{}", member.organization_id),
            &Flash::Ok("You have joined.".into()),
        ),
        Err(error) => flash_to(&back, &Flash::Error(message(&error))),
    }
}

async fn signed_in<S>(state: &UiState<S>, headers: &HeaderMap) -> bool
where
    S: AuthStorage,
{
    let Some(token) = token_of(headers, &state.cookie) else {
        return false;
    };
    state
        .auth
        .current_session(CurrentSession { token })
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn a_name_becomes_a_slug_without_doubled_dashes() {
        assert_eq!(slugify("Acme Records, Inc."), "acme-records-inc");
        assert_eq!(slugify("  FastTrack  Studio  "), "fasttrack-studio");
    }

    #[test]
    fn a_slug_never_starts_or_ends_with_a_dash() {
        // `normalize_slug` would reject neither, but a leading dash in a
        // URL reads as a typo and a trailing one as a truncation.
        assert_eq!(slugify("…Ada!"), "ada");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn non_ascii_does_not_survive_into_a_url() {
        assert_eq!(slugify("Café Ædifice"), "caf-difice");
    }
}
