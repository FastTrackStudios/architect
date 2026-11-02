//! [`SpacesEngine`]: the gate's engine for what spaces give.

use architect_orgs::OrgId;
use architect_orgs::server::current_org;
use architect_permissions::{Action, Decision, PermissionEngine, Principal, Resource};

use super::{Spaces, now_us};
use crate::model::Subject;

/// What the org's spaces give the caller, as a [`PermissionEngine`].
///
/// It only ever allows what a space gives; everything else it denies, so
/// it sits **beside** the org's engine, not in place of it — the gate's
/// engine is `CompositeEngine::new().push(org_engine).push(spaces_engine)`
/// (first allow wins): effective access is the org role ∪ the space
/// role, capped by the org's [`SpacePolicy`](crate::SpacePolicy).
///
/// The org a check is in is the resource's (`org/<id>/…`) or else the
/// call's (`current_org`), as for `ConfinedEngine`; a resource in another
/// org than the call's is refused. Its state is read on every check, so
/// a removal binds on the caller's next call.
#[derive(Clone)]
pub struct SpacesEngine {
    spaces: Spaces,
}

impl SpacesEngine {
    pub(super) const fn new(spaces: Spaces) -> Self {
        Self { spaces }
    }
}

/// The org a check is in and the org-relative resource, or why there is
/// none.
fn place(what: &Resource) -> Result<(OrgId, String), String> {
    let named = OrgId::of_path(what.as_str());
    let current = current_org();
    match (named, current) {
        (Some((named, _)), Some(current)) if named != current => {
            Err(format!("{} is outside org {}", what.as_str(), current.0))
        }
        (Some((org, rest)), _) => Ok((org, rest.to_owned())),
        (None, Some(org)) => Ok((org, what.as_str().to_owned())),
        (None, None) => Err(format!("{} is in no org", what.as_str())),
    }
}

impl PermissionEngine for SpacesEngine {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        let (org, relative) = match place(what) {
            Ok(place) => place,
            Err(reason) => return Decision::deny(reason),
        };
        let Some(subject) = Subject::of(who) else {
            return Decision::deny(format!("{} is in no space", who.describe()));
        };
        let standing = self.spaces.standing(who, &subject, &org);
        let state = self.spaces.snapshot(&org);
        if state.allows(&subject, &standing, &relative, action.as_str(), now_us()) {
            Decision::Allow
        } else {
            Decision::deny(format!(
                "no space of org {} gives {} {} on {relative}",
                org.0,
                who.describe(),
                action.as_str()
            ))
        }
    }

    /// The caller's rules across the org's spaces, org-relative.
    fn survey(&self, who: &Principal, prefix: &Resource) -> Vec<(Resource, Vec<Action>)> {
        let (Ok((org, prefix)), Some(subject)) = (place(prefix), Subject::of(who)) else {
            return Vec::new();
        };
        let standing = self.spaces.standing(who, &subject, &org);
        let state = self.spaces.snapshot(&org);
        state
            .holdings(&subject, &standing, now_us())
            .into_iter()
            .flat_map(|h| h.rules)
            .filter(|r| prefix.is_empty() || r.resource.starts_with(&prefix))
            .map(|r| {
                (
                    Resource::new(r.resource),
                    r.actions.into_iter().map(Action::new).collect(),
                )
            })
            .collect()
    }
}
