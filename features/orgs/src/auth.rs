//! The identity server's answer (feature `auth`): auth-proto's
//! `OrganizationBundle`s — one per org the session belongs to, with its
//! membership — as [`Org`]s, and [`fetch`], which asks for them with the
//! session auth-client keeps.

use auth_client::StoredSession;
use auth_proto::{AuthFlowError, OrganizationBundle, OrganizationService};

use crate::model::{Org, OrgId, Role};

/// The palette a switcher's marks take, by the org's id.
const MARKS: [&str; 6] = [
    "#1d4ed8", "#b45309", "#047857", "#7c3aed", "#be123c", "#0e7490",
];

/// The person's orgs from `OrganizationService::list_organizations`.
/// `personal` names the person's own org, when the app knows it (its
/// slug is the user's id, by fts-auth's convention).
#[must_use]
pub fn from_bundles(bundles: &[OrganizationBundle], personal: Option<&str>) -> Vec<Org> {
    bundles
        .iter()
        .map(|b| {
            let id = b.organization.id.to_string();
            let mark = id
                .bytes()
                .fold(0usize, |a, c| {
                    a.wrapping_mul(31).wrapping_add(usize::from(c))
                })
                .checked_rem(MARKS.len())
                .unwrap_or(0);
            Org {
                id: OrgId::new(id.clone()),
                name: b.organization.name.clone(),
                slug: b.organization.slug.clone(),
                role: Role::parse(&b.membership.role),
                personal: personal.is_some_and(|p| p == b.organization.slug || p == id),
                members: 1,
                color: MARKS.get(mark).copied().unwrap_or("#3f3f46").to_owned(),
            }
        })
        .collect()
}

/// The signed-in person's orgs, live from the identity server.
///
/// `OrganizationService::list_organizations` over any of its clients (vox
/// or HTTP), with the session auth-client stored. The person's own org is
/// the one whose slug is their user id.
///
/// # Errors
///
/// The identity server's: `SessionExpired` when the session is no longer
/// valid (sign in again), anything else when it could not answer (keep
/// the cached list).
pub async fn fetch<C: OrganizationService>(
    client: &C,
    session: &StoredSession,
) -> Result<Vec<Org>, AuthFlowError> {
    let bundles = client.list_organizations(session.token.clone()).await?;
    Ok(from_bundles(&bundles, session.user_id.as_deref()))
}
