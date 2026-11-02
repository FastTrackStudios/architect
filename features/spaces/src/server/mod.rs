//! Every org's spaces, served (feature `server`, native).
//!
//! [`Spaces`] keeps them — read from a [`SpaceStore`] once at open,
//! answered from memory, written through on every change with an audit
//! entry first — and is three things a server mounts:
//!
//! - the gate's engine for spaces, [`Spaces::engine`] ([`SpacesEngine`]),
//!   beside the org's engine in a `CompositeEngine`, so every gated
//!   service, `DocPermissions` and the Action List see what spaces give;
//! - the files feature's grant source (feature `files`): a space over a
//!   folder grants its members that folder, generated, never hand-made;
//! - [`SpacesService`] for the gate's caller in the lane's org.
//!
//! ```ignore
//! let members = Members::new();
//! let spaces = Spaces::open(FileStore::new(&data_dir), members.clone()).await?;
//! let engine = CompositeEngine::new()
//!     .push(Arc::new(ConfinedEngine::new(members.clone()).role(…)))
//!     .push(Arc::new(spaces.engine()));
//! let files = FilesServer::new(&data_dir, blobs, members.clone()).with_grants(spaces.clone());
//! let gate = PermissionsGate::new(Arc::new(engine), Arc::new(identity))
//!     .permit(spaces_service_rpc_service_descriptor(), &architect_spaces::PERMITS)
//!     .permit(…);
//! let router = LayerRouter::new()
//!     .with(spaces_service_rpc_service_descriptor(), architect_spaces::serve(spaces.clone()));
//! // per connection, at `/org/{slug}/vox`:
//! let handler = confine(org, PermissionsGate::wrap_shared_with_bearer(gate, router, bearer));
//! ```

mod engine;
#[cfg(feature = "files")]
mod files;
mod store;

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use architect_orgs::server::Members;
use architect_orgs::{OrgId, Role};
use architect_permissions::Principal;

use crate::model::{
    MANAGER, Member, NewSpace, OrgSpaces, Space, SpaceAudit, SpaceChange, SpaceGrant, SpaceId,
    SpacePatch, SpacePolicy, SpaceRole, SpaceSummary, SpacesFault, Standing, Subject,
    builtin_roles, validate_pattern, validate_reach,
};
use crate::service::SpacesService;

pub use engine::SpacesEngine;
pub use store::{FileStore, MemoryStore, SpaceStore, StoreFuture};

/// Every org's spaces. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct Spaces {
    inner: Arc<Inner>,
    default_org: Option<OrgId>,
}

struct Inner {
    store: Arc<dyn SpaceStore>,
    members: Members,
    orgs: RwLock<HashMap<OrgId, Arc<OrgSpaces>>>,
    /// One change at a time: each reads the state, writes it through and
    /// swaps it in.
    writing: tokio::sync::Mutex<()>,
}

impl Spaces {
    /// Read every org's spaces from `store`; org roles from `members`
    /// (the gate's `MembersResolver` fills it).
    pub async fn open(
        store: impl SpaceStore + 'static,
        members: Members,
    ) -> Result<Self, SpacesFault> {
        let loaded = store.load().await?;
        Ok(Self {
            inner: Arc::new(Inner {
                store: Arc::new(store),
                members,
                orgs: RwLock::new(
                    loaded
                        .into_iter()
                        .map(|(org, spaces)| (org, Arc::new(spaces)))
                        .collect(),
                ),
                writing: tokio::sync::Mutex::new(()),
            }),
            default_org: None,
        })
    }

    /// The org a call on a lane in no org is in: a server that serves one
    /// org. Without it such a call is [`SpacesFault::NoOrg`].
    #[must_use]
    pub fn default_org(mut self, org: OrgId) -> Self {
        self.default_org = Some(org);
        self
    }

    /// The gate's engine for what spaces give.
    #[must_use]
    pub fn engine(&self) -> SpacesEngine {
        SpacesEngine::new(self.clone())
    }

    /// The service for `who` in `org`, called directly (a server's own
    /// job, a test).
    #[must_use]
    pub fn acting(&self, org: OrgId, who: Principal) -> Acting {
        Acting {
            spaces: self.clone(),
            org,
            who,
        }
    }

    /// `org`'s spaces as they are now.
    #[must_use]
    pub fn snapshot(&self, org: &OrgId) -> Arc<OrgSpaces> {
        self.inner
            .orgs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(org)
            .cloned()
            .unwrap_or_default()
    }

    /// How `subject` (the principal `who`) stands to `org`.
    fn standing(&self, who: &Principal, subject: &Subject, org: &OrgId) -> Standing {
        let role = match subject {
            Subject::User(_) => self.inner.members.role(who, org),
            Subject::Guest(_) => None,
        };
        Standing::of(subject, role)
    }

    /// The service for the call running now: its lane's org (or the
    /// default) and the gate's principal.
    fn for_call(&self) -> Result<Acting, SpacesFault> {
        let org = architect_orgs::server::current_org()
            .or_else(|| self.default_org.clone())
            .ok_or(SpacesFault::NoOrg)?;
        let who = architect::permissions_gate::caller().ok_or(SpacesFault::Unauthenticated)?;
        Ok(self.acting(org, who))
    }

    /// Change `org`'s spaces: `f` works on a copy and says what it
    /// changed; the change is audited, the copy written through, then
    /// swapped in, so the next check anywhere sees it.
    async fn mutate<T>(
        &self,
        org: &OrgId,
        by: String,
        f: impl FnOnce(&mut OrgSpaces, i64) -> Result<(T, Option<SpaceId>, SpaceChange), SpacesFault>,
    ) -> Result<T, SpacesFault> {
        let _writing = self.inner.writing.lock().await;
        let now = now_us();
        let mut next = OrgSpaces::clone(&self.snapshot(org));
        let (out, space, change) = f(&mut next, now)?;
        let entry = SpaceAudit {
            at_us: now,
            space,
            by,
            change,
        };
        self.inner.store.record(org, &entry).await?;
        self.inner.store.save(org, &next).await?;
        self.inner
            .orgs
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(org.clone(), Arc::new(next));
        Ok(out)
    }
}

/// Who a call is for, as spaces see them.
enum Caller {
    /// An in-process service: may do anything.
    Service,
    Person(Subject, Standing),
}

/// [`SpacesService`] for one principal in one org ([`Spaces::acting`]).
#[derive(Clone)]
pub struct Acting {
    spaces: Spaces,
    org: OrgId,
    who: Principal,
}

impl Acting {
    fn caller(&self) -> Result<Caller, SpacesFault> {
        if matches!(self.who, Principal::Service { .. }) {
            return Ok(Caller::Service);
        }
        let subject = Subject::of(&self.who).ok_or(SpacesFault::Unauthenticated)?;
        let standing = self.spaces.standing(&self.who, &subject, &self.org);
        Ok(Caller::Person(subject, standing))
    }

    /// Whether the caller manages `space`: the org's owners and admins,
    /// and whoever holds `admin` on the space after the org's policy.
    fn manages(caller: &Caller, state: &OrgSpaces, space: &Space, now: i64) -> bool {
        match caller {
            Caller::Service => true,
            Caller::Person(_, standing) if standing.stewards() => true,
            Caller::Person(subject, standing) => {
                state.allows(subject, standing, &space.id.resource(), "admin", now)
            }
        }
    }

    /// Whether the caller may see `space`: its managers and members.
    fn sees(caller: &Caller, state: &OrgSpaces, space: &Space, now: i64) -> bool {
        Self::manages(caller, state, space, now)
            || matches!(caller, Caller::Person(subject, standing)
                if state.holdings(subject, standing, now).iter().any(|h| h.space.id == space.id))
    }

    /// `id`, if the caller may see it (and manage it, with `manage`): a
    /// space they may not see is not found, one they may see and not
    /// manage is denied.
    fn find<'s>(
        caller: &Caller,
        state: &'s OrgSpaces,
        id: &SpaceId,
        manage: bool,
        now: i64,
    ) -> Result<&'s Space, SpacesFault> {
        let space = state
            .space(id)
            .filter(|s| Self::sees(caller, state, s, now))
            .ok_or_else(|| SpacesFault::NotFound(id.clone()))?;
        if manage && !Self::manages(caller, state, space, now) {
            return Err(SpacesFault::Denied(format!("managing space {id}")));
        }
        Ok(space)
    }

    /// Change one space the caller manages.
    async fn change<T>(
        &self,
        id: SpaceId,
        f: impl FnOnce(&mut Space, &SpacePolicy, i64) -> Result<(T, SpaceChange), SpacesFault>,
    ) -> Result<T, SpacesFault> {
        let caller = self.caller()?;
        self.spaces
            .mutate(&self.org, self.who.describe(), |state, now| {
                Self::find(&caller, state, &id, true, now)?;
                let policy = state.policy.clone();
                let space = state
                    .space_mut(&id)
                    .ok_or_else(|| SpacesFault::NotFound(id.clone()))?;
                let (out, change) = f(space, &policy, now)?;
                Ok((out, Some(id.clone()), change))
            })
            .await
    }

    fn summary(caller: &Caller, space: &Space) -> SpaceSummary {
        let role = match caller {
            Caller::Person(subject, _) => space.member(subject).and_then(|m| m.role.clone()),
            Caller::Service => None,
        };
        SpaceSummary {
            id: space.id.clone(),
            name: space.name.clone(),
            kind: space.kind.clone(),
            role,
            members: u32::try_from(space.members.len()).unwrap_or(u32::MAX),
        }
    }
}

fn require_name(name: &str) -> Result<String, SpacesFault> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SpacesFault::Invalid("a name".into()));
    }
    Ok(name.to_owned())
}

fn require_actions(actions: &[String]) -> Result<(), SpacesFault> {
    if actions.is_empty() || actions.iter().any(|a| a.trim().is_empty()) {
        return Err(SpacesFault::Invalid(
            "at least one action, none empty".into(),
        ));
    }
    Ok(())
}

impl SpacesService for Acting {
    async fn create_space(&self, new: NewSpace) -> Result<Space, SpacesFault> {
        let caller = self.caller()?;
        let name = require_name(&new.name)?;
        validate_reach(&new.scope, &new.exclude)?;
        let by = self.who.describe();
        let org = self.org.clone();
        self.spaces
            .mutate(&self.org, by.clone(), move |state, now| {
                let creator = match &caller {
                    Caller::Service => None,
                    Caller::Person(subject, Standing::Member(role)) => {
                        let may = match role {
                            Role::Owner | Role::Admin => true,
                            Role::Member => state.policy.members_create,
                            Role::Viewer | Role::Other(_) => false,
                        };
                        if !may {
                            return Err(SpacesFault::Denied(format!(
                                "a {} of the org creating a space",
                                role.label()
                            )));
                        }
                        Some(subject.clone())
                    }
                    Caller::Person(..) => {
                        return Err(SpacesFault::Denied(
                            "creating a space in an org one is not in".into(),
                        ));
                    }
                };
                let space = Space {
                    id: SpaceId::new(uuid::Uuid::new_v4().to_string()),
                    org,
                    name: name.clone(),
                    kind: new.kind.trim().to_owned(),
                    scope: new.scope.clone(),
                    exclude: new.exclude,
                    roles: Vec::new(),
                    members: creator
                        .into_iter()
                        .map(|subject| Member {
                            subject,
                            role: Some(MANAGER.to_owned()),
                            grants: Vec::new(),
                            added_by: by.clone(),
                            added_us: now,
                            expires_us: None,
                        })
                        .collect(),
                    created_by: by,
                    created_us: now,
                };
                state.spaces.push(space.clone());
                let change = SpaceChange::Created {
                    name,
                    scope: new.scope,
                };
                Ok((space.clone(), Some(space.id), change))
            })
            .await
    }

    async fn spaces(&self) -> Result<Vec<SpaceSummary>, SpacesFault> {
        let caller = self.caller()?;
        let state = self.spaces.snapshot(&self.org);
        let now = now_us();
        Ok(state
            .spaces
            .iter()
            .filter(|s| Self::sees(&caller, &state, s, now))
            .map(|s| Self::summary(&caller, s))
            .collect())
    }

    async fn space(&self, id: SpaceId) -> Result<Space, SpacesFault> {
        let caller = self.caller()?;
        let state = self.spaces.snapshot(&self.org);
        Self::find(&caller, &state, &id, false, now_us()).cloned()
    }

    async fn update_space(&self, id: SpaceId, patch: SpacePatch) -> Result<Space, SpacesFault> {
        self.change(id, move |space, _, _| {
            if let Some(name) = &patch.name {
                space.name = require_name(name)?;
            }
            if let Some(scope) = &patch.scope {
                space.scope.clone_from(scope);
            }
            if let Some(exclude) = &patch.exclude {
                space.exclude.clone_from(exclude);
            }
            validate_reach(&space.scope, &space.exclude)?;
            // What the space no longer covers, no grant in it reaches.
            let covered = space.clone();
            for member in &mut space.members {
                member.grants.retain(|g| covered.covers(&g.resource));
            }
            space
                .members
                .retain(|m| m.role.is_some() || !m.grants.is_empty());
            Ok((space.clone(), SpaceChange::Updated { patch }))
        })
        .await
    }

    async fn delete_space(&self, id: SpaceId) -> Result<(), SpacesFault> {
        let caller = self.caller()?;
        self.spaces
            .mutate(&self.org, self.who.describe(), |state, now| {
                Self::find(&caller, state, &id, true, now)?;
                state.spaces.retain(|s| s.id != id);
                Ok(((), Some(id.clone()), SpaceChange::Deleted))
            })
            .await
    }

    async fn set_member(
        &self,
        id: SpaceId,
        subject: Subject,
        role: Option<String>,
        expires_us: Option<i64>,
    ) -> Result<Member, SpacesFault> {
        let by = self.who.describe();
        self.change(id, move |space, policy, now| {
            if let Some(role) = &role
                && space.role_actions(role).is_none()
            {
                return Err(SpacesFault::Invalid(format!(
                    "no role {role} in this space"
                )));
            }
            if matches!(subject, Subject::Guest(_)) && !policy.guests {
                return Err(SpacesFault::Policy("the org admits no link guests".into()));
            }
            let member =
                if let Some(member) = space.members.iter_mut().find(|m| m.subject == subject) {
                    member.role.clone_from(&role);
                    member.expires_us = expires_us;
                    member.clone()
                } else {
                    let member = Member {
                        subject: subject.clone(),
                        role: role.clone(),
                        grants: Vec::new(),
                        added_by: by,
                        added_us: now,
                        expires_us,
                    };
                    space.members.push(member.clone());
                    member
                };
            Ok((member, SpaceChange::MemberSet { subject, role }))
        })
        .await
    }

    async fn remove_member(&self, id: SpaceId, subject: Subject) -> Result<(), SpacesFault> {
        self.change(id, move |space, _, _| {
            let before = space.members.len();
            space.members.retain(|m| m.subject != subject);
            if space.members.len() == before {
                return Err(SpacesFault::Invalid(format!(
                    "{} is not a member",
                    subject.describe()
                )));
            }
            Ok(((), SpaceChange::MemberRemoved { subject }))
        })
        .await
    }

    async fn grant(
        &self,
        id: SpaceId,
        subject: Subject,
        grant: SpaceGrant,
    ) -> Result<Member, SpacesFault> {
        validate_pattern(&grant.resource)?;
        require_actions(&grant.actions)?;
        let by = self.who.describe();
        self.change(id, move |space, policy, now| {
            if !space.covers(&grant.resource) {
                return Err(SpacesFault::Invalid(format!(
                    "{} is not inside the space",
                    grant.resource
                )));
            }
            if matches!(subject, Subject::Guest(_)) && !policy.guests {
                return Err(SpacesFault::Policy("the org admits no link guests".into()));
            }
            if !space.members.iter().any(|m| m.subject == subject) {
                space.members.push(Member {
                    subject: subject.clone(),
                    role: None,
                    grants: Vec::new(),
                    added_by: by,
                    added_us: now,
                    expires_us: None,
                });
            }
            let member = space
                .members
                .iter_mut()
                .find(|m| m.subject == subject)
                .ok_or_else(|| SpacesFault::Internal("the member just added".into()))?;
            member.grants.retain(|g| g.resource != grant.resource);
            member.grants.push(grant.clone());
            Ok((member.clone(), SpaceChange::Granted { subject, grant }))
        })
        .await
    }

    async fn revoke_grant(
        &self,
        id: SpaceId,
        subject: Subject,
        resource: String,
    ) -> Result<Member, SpacesFault> {
        self.change(id, move |space, _, _| {
            let not_held = || {
                SpacesFault::Invalid(format!(
                    "{} holds no grant on {resource}",
                    subject.describe()
                ))
            };
            let member = space
                .members
                .iter_mut()
                .find(|m| m.subject == subject)
                .ok_or_else(not_held)?;
            let before = member.grants.len();
            member.grants.retain(|g| g.resource != resource);
            if member.grants.len() == before {
                return Err(not_held());
            }
            let member = member.clone();
            space
                .members
                .retain(|m| m.role.is_some() || !m.grants.is_empty());
            Ok((member, SpaceChange::GrantRevoked { subject, resource }))
        })
        .await
    }

    async fn set_role(&self, id: SpaceId, role: SpaceRole) -> Result<Space, SpacesFault> {
        let name = require_name(&role.name)?;
        require_actions(&role.actions)?;
        let role = SpaceRole { name, ..role };
        self.change(id, move |space, _, _| {
            space.roles.retain(|r| r.name != role.name);
            space.roles.push(role.clone());
            Ok((space.clone(), SpaceChange::RoleSet { role }))
        })
        .await
    }

    async fn remove_role(&self, id: SpaceId, name: String) -> Result<Space, SpacesFault> {
        self.change(id, move |space, _, _| {
            if !space.roles.iter().any(|r| r.name == name) {
                return Err(SpacesFault::Invalid(format!(
                    "{name} is not a role of this space's own"
                )));
            }
            let builtin = builtin_roles().iter().any(|r| r.name == name);
            if !builtin
                && space
                    .members
                    .iter()
                    .any(|m| m.role.as_deref() == Some(name.as_str()))
            {
                return Err(SpacesFault::Invalid(format!("a member holds {name}")));
            }
            space.roles.retain(|r| r.name != name);
            Ok((space.clone(), SpaceChange::RoleRemoved { name }))
        })
        .await
    }

    async fn effective(&self, id: SpaceId) -> Result<Vec<SpaceGrant>, SpacesFault> {
        let caller = self.caller()?;
        let state = self.spaces.snapshot(&self.org);
        let now = now_us();
        let space = Self::find(&caller, &state, &id, false, now)?;
        let rules = match &caller {
            Caller::Service => space
                .scope
                .iter()
                .chain(std::iter::once(&space.id.resource()))
                .map(|s| SpaceGrant::new(s.clone(), &["*"]))
                .collect(),
            Caller::Person(subject, standing) => state
                .holdings(subject, standing, now)
                .into_iter()
                .filter(|h| h.space.id == id)
                .flat_map(|h| h.rules)
                .map(|r| SpaceGrant {
                    resource: r.resource,
                    actions: r.actions,
                })
                .collect(),
        };
        Ok(rules)
    }

    async fn audit(&self, id: SpaceId) -> Result<Vec<SpaceAudit>, SpacesFault> {
        let caller = self.caller()?;
        {
            let state = self.spaces.snapshot(&self.org);
            Self::find(&caller, &state, &id, true, now_us())?;
        }
        self.spaces.inner.store.audit(&self.org, &id).await
    }

    async fn policy(&self) -> Result<SpacePolicy, SpacesFault> {
        match self.caller()? {
            Caller::Service | Caller::Person(_, Standing::Member(_)) => {
                Ok(self.spaces.snapshot(&self.org).policy.clone())
            }
            Caller::Person(..) => Err(SpacesFault::Denied(
                "the policy of an org one is not in".into(),
            )),
        }
    }

    async fn set_policy(&self, policy: SpacePolicy) -> Result<SpacePolicy, SpacesFault> {
        let steward = match self.caller()? {
            Caller::Service => true,
            Caller::Person(_, standing) => standing.stewards(),
        };
        if !steward {
            return Err(SpacesFault::Denied(
                "the org's owners and admins set its policy".into(),
            ));
        }
        self.spaces
            .mutate(&self.org, self.who.describe(), move |state, _| {
                state.policy = policy.clone();
                Ok((policy.clone(), None, SpaceChange::PolicySet { policy }))
            })
            .await
    }
}

/// The service for the call running now: its lane's org and the gate's
/// caller.
impl SpacesService for Spaces {
    async fn create_space(&self, new: NewSpace) -> Result<Space, SpacesFault> {
        self.for_call()?.create_space(new).await
    }
    async fn spaces(&self) -> Result<Vec<SpaceSummary>, SpacesFault> {
        self.for_call()?.spaces().await
    }
    async fn space(&self, id: SpaceId) -> Result<Space, SpacesFault> {
        self.for_call()?.space(id).await
    }
    async fn update_space(&self, id: SpaceId, patch: SpacePatch) -> Result<Space, SpacesFault> {
        self.for_call()?.update_space(id, patch).await
    }
    async fn delete_space(&self, id: SpaceId) -> Result<(), SpacesFault> {
        self.for_call()?.delete_space(id).await
    }
    async fn set_member(
        &self,
        id: SpaceId,
        subject: Subject,
        role: Option<String>,
        expires_us: Option<i64>,
    ) -> Result<Member, SpacesFault> {
        self.for_call()?
            .set_member(id, subject, role, expires_us)
            .await
    }
    async fn remove_member(&self, id: SpaceId, subject: Subject) -> Result<(), SpacesFault> {
        self.for_call()?.remove_member(id, subject).await
    }
    async fn grant(
        &self,
        id: SpaceId,
        subject: Subject,
        grant: SpaceGrant,
    ) -> Result<Member, SpacesFault> {
        self.for_call()?.grant(id, subject, grant).await
    }
    async fn revoke_grant(
        &self,
        id: SpaceId,
        subject: Subject,
        resource: String,
    ) -> Result<Member, SpacesFault> {
        self.for_call()?.revoke_grant(id, subject, resource).await
    }
    async fn set_role(&self, id: SpaceId, role: SpaceRole) -> Result<Space, SpacesFault> {
        self.for_call()?.set_role(id, role).await
    }
    async fn remove_role(&self, id: SpaceId, name: String) -> Result<Space, SpacesFault> {
        self.for_call()?.remove_role(id, name).await
    }
    async fn effective(&self, id: SpaceId) -> Result<Vec<SpaceGrant>, SpacesFault> {
        self.for_call()?.effective(id).await
    }
    async fn audit(&self, id: SpaceId) -> Result<Vec<SpaceAudit>, SpacesFault> {
        self.for_call()?.audit(id).await
    }
    async fn policy(&self) -> Result<SpacePolicy, SpacesFault> {
        self.for_call()?.policy().await
    }
    async fn set_policy(&self, policy: SpacePolicy) -> Result<SpacePolicy, SpacesFault> {
        self.for_call()?.set_policy(policy).await
    }
}

/// Microseconds since the Unix epoch.
pub(crate) fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
}
