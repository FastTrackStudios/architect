//! Every service confined to the caller's org (feature `server`, native).
//!
//! Three pieces, mounted around the `PermissionsGate` the app already has
//! (`architect::permissions_gate`):
//!
//! - An [`OrgId`] is a lane's scope (`architect::permissions_gate::LaneScope`):
//!   on an `EngineHost`, `vox_at`'s resolver returns
//!   `Lane::new(router).scoped(org)` for a per-org lane (`/org/{slug}/vox`)
//!   and the host enters the org outside its gate; on a transport the app
//!   serves itself, [`confine`] does the same to a gated handler. Every
//!   call, the gate's check included, then runs in that org:
//!   [`current_org`] answers it, and services key their data by it
//!   ([`OrgId::path`]).
//! - [`MembersResolver`] wraps the gate's identity resolver: when it
//!   resolves a session it also learns, from the identity server
//!   ([`Directory`]), which orgs that person is in and their role in each,
//!   into [`Members`].
//! - [`ConfinedEngine`] is the gate's engine: a call in an org the caller is
//!   not a member of is refused, so is a resource in another org
//!   (`org/<other>/…`), and a role's rules (org-relative) decide the rest.
//!
//! ```ignore
//! let members = Members::new();
//! let identity = MembersResolver::new(sessions, IdentityServer(orgs_client), members.clone());
//! let engine = ConfinedEngine::new(members.clone())
//!     .role(&Role::Viewer, vec![Rule::new("**", &["read"])])
//!     .role(&Role::Member, vec![Rule::new("**", &["read", "write"])]);
//! let gate = PermissionsGate::new(Arc::new(engine), Arc::new(identity)).permit(…);
//! // an `EngineHost` lane, `/org/{slug}/vox`:
//! EngineHost::new(router.clone(), gate, bind).vox_at("/org/{slug}/vox", move |head| {
//!     let router = router.clone();
//!     async move { Ok::<_, StatusCode>(Lane::new(router).scoped(org_of(&head)?)) }
//! });
//! // or per connection, on a transport of your own:
//! let handler = confine(org_id, PermissionsGate::wrap_shared_with_bearer(gate, router, bearer));
//! ```
//!
//! A lane in no org (`/vox`) is the All-organizations one: a resource
//! there names its org (`org/<id>/…`) and needs the caller's membership
//! of it, and a service reads across [`Members::scope_of`] the caller. A
//! gate with no such lane sets [`ConfinedEngine::require_org`].

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use architect::permissions_gate::{LaneScope, PermissionedRouter, Scoped};
use architect_permissions::{
    Action, BoxIdentityFuture, Decision, IdentityResolver, PermissionEngine, Principal, Resource,
    Rule,
};

use crate::model::{Org, OrgId, Role};
use crate::scope::Scope;

tokio::task_local! {
    /// The org the call running on this task is confined to.
    static CURRENT: OrgId;
}

/// The org this call is confined to, on a lane scoped to an org (an
/// `EngineHost` lane's, or [`confine`]'s). `None` on a lane in no org (the All-organizations one), or off the
/// request task.
#[must_use]
pub fn current_org() -> Option<OrgId> {
    CURRENT.try_with(Clone::clone).ok()
}

/// Runs `fut` confined to `org`: what [`confine`] does for each call, for
/// work a handler spawns or a test drives directly.
pub async fn in_org<F: Future>(org: OrgId, fut: F) -> F::Output {
    CURRENT.scope(org, fut).await
}

/// A gated connection, every call on it (the gate's check included)
/// confined to one org: what [`confine`] returns.
pub type OrgConfined<H> = Scoped<H, OrgId>;

/// Confines every call `gated` handles to `org`, outside the gate.
///
/// The org is entered before the check, so the gate's engine
/// ([`ConfinedEngine`]) checks the call in it. Once per connection, on a
/// transport the app serves itself; on an
/// `EngineHost` lane, return `Lane::new(router).scoped(org)` from
/// `vox_at`'s resolver instead and the host does the same.
///
/// Only a gated handler fits: confining inside the gate, where the check
/// cannot see the org, does not compile.
#[must_use]
pub const fn confine<H>(org: OrgId, gated: PermissionedRouter<H>) -> OrgConfined<H> {
    gated.scoped(org)
}

/// An org is a lane's scope: every call on the lane runs in it
/// ([`current_org`]).
impl LaneScope for OrgId {
    fn enter<F>(&self, call: F) -> impl Future<Output = F::Output> + Send
    where
        F: Future + Send,
    {
        CURRENT.scope(self.clone(), call)
    }
}

/// Who is in which org, with what role: learnt from the identity server
/// as sessions resolve ([`MembersResolver`]), or set by the deployment (a
/// peer host admitted to an org).
#[derive(Clone, Default)]
pub struct Members {
    by_principal: Arc<RwLock<HashMap<String, Vec<Org>>>>,
}

impl Members {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `who` is in exactly `orgs`.
    pub fn set(&self, who: &Principal, orgs: Vec<Org>) {
        self.by_principal
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(who.describe(), orgs);
    }

    /// `who` is in no org any more.
    pub fn forget(&self, who: &Principal) {
        self.by_principal
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&who.describe());
    }

    /// The orgs `who` is in.
    #[must_use]
    pub fn orgs_of(&self, who: &Principal) -> Vec<Org> {
        self.by_principal
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&who.describe())
            .cloned()
            .unwrap_or_default()
    }

    /// `who`'s role in `org`, if they are in it.
    #[must_use]
    pub fn role(&self, who: &Principal, org: &OrgId) -> Option<Role> {
        self.by_principal
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&who.describe())
            .and_then(|orgs| orgs.iter().find(|o| &o.id == org))
            .map(|o| o.role.clone())
    }

    /// Every org `who` is in: what an All-organizations read on the
    /// server covers.
    #[must_use]
    pub fn scope_of(&self, who: &Principal) -> Scope {
        Scope::All(self.orgs_of(who).into_iter().map(|o| o.id).collect())
    }
}

/// A boxed answer from a [`Directory`].
pub type BoxOrgsFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<Org>, String>> + Send + 'a>>;

/// Where a session's orgs come from: the identity server
/// ([`IdentityServer`], feature `auth`), or a fixed table in a test.
pub trait Directory: Send + Sync {
    /// The orgs the session `token` belongs to, with its role in each.
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a>;
}

/// The identity server as a [`Directory`]: any `OrganizationService`
/// client (vox or HTTP) pointed at fts-auth.
#[cfg(feature = "auth")]
pub struct IdentityServer<C>(pub C);

#[cfg(feature = "auth")]
impl<C> Directory for IdentityServer<C>
where
    C: auth_proto::OrganizationService + Send + Sync,
{
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a> {
        Box::pin(async move {
            self.0
                .list_organizations(token.to_owned())
                .await
                .map(|bundles| crate::auth::from_bundles(&bundles, None))
                .map_err(|e| e.to_string())
        })
    }
}

/// How long a session's orgs are trusted before the identity server is
/// asked again: a removal reaches every app within it.
pub const MEMBERS_TTL: Duration = Duration::from_secs(60);

/// The gate's identity resolver, learning each session's orgs as it
/// resolves it.
///
/// The principal is `inner`'s; a session is asked about at
/// most once per [`MEMBERS_TTL`] (`with_ttl`). When the identity server
/// cannot answer, the last answer stands; a person it never answered for
/// is in no org (refused, not waved through).
pub struct MembersResolver<R> {
    inner: R,
    directory: Arc<dyn Directory>,
    members: Members,
    ttl: Duration,
    asked: Mutex<HashMap<String, Instant>>,
}

impl<R> MembersResolver<R> {
    pub fn new(inner: R, directory: impl Directory + 'static, members: Members) -> Self {
        Self {
            inner,
            directory: Arc::new(directory),
            members,
            ttl: MEMBERS_TTL,
            asked: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub const fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Whether `token`'s orgs are due, marking them asked now if so.
    fn due(&self, token: &str, now: Instant) -> bool {
        let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        if asked
            .get(token)
            .is_some_and(|at| now.saturating_duration_since(*at) < self.ttl)
        {
            return false;
        }
        asked.retain(|_, at| now.saturating_duration_since(*at) < self.ttl);
        asked.insert(token.to_owned(), now);
        true
    }
}

impl<R: IdentityResolver> IdentityResolver for MembersResolver<R> {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        Box::pin(async move {
            let who = self.inner.resolve(bearer_token).await;
            let Some(token) = bearer_token else {
                return who;
            };
            if matches!(who, Principal::Anonymous) || !self.due(token, Instant::now()) {
                return who;
            }
            if let Ok(orgs) = self.directory.orgs(token).await {
                self.members.set(&who, orgs);
            }
            who
        })
    }
}

/// The gate's engine for org-confined services: membership first, then
/// the caller's role in that org, then `inner` (default: allow).
///
/// The org a check is in is the resource's (`org/<id>/…`) or else the
/// call's ([`current_org`]); a resource in another org than the call's is
/// refused outright. A check in no org is `inner`'s alone, unless the
/// engine [requires an org](Self::require_org). An in-process service
/// (`Principal::Service`) is in every org.
pub struct ConfinedEngine {
    members: Members,
    roles: HashMap<String, Vec<Rule>>,
    inner: Option<Arc<dyn PermissionEngine>>,
    require_org: bool,
}

impl ConfinedEngine {
    #[must_use]
    pub fn new(members: Members) -> Self {
        Self {
            members,
            roles: HashMap::new(),
            inner: None,
            require_org: false,
        }
    }

    /// Every check must be in an org: a call on a lane in no org, on a
    /// resource naming none, is refused rather than passed to `inner`.
    /// For a gate that serves only org lanes (a gate per org, or a host
    /// with no All-organizations lane), so a lane left unscoped fails
    /// closed instead of reading as the All-organizations one.
    #[must_use]
    pub const fn require_org(mut self) -> Self {
        self.require_org = true;
        self
    }

    /// What `role` may do in an org, by rules on org-relative resources
    /// (`vault/**`, not `org/<id>/vault/**`). Once any role has rules, a
    /// role with none may do nothing.
    #[must_use]
    pub fn role(mut self, role: &Role, rules: Vec<Rule>) -> Self {
        self.roles.insert(role.label(), rules);
        self
    }

    /// Asks `inner` too, after confinement and the role allowed it.
    #[must_use]
    pub fn over(mut self, inner: Arc<dyn PermissionEngine>) -> Self {
        self.inner = Some(inner);
        self
    }

    fn rest(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        self.inner
            .as_ref()
            .map_or(Decision::Allow, |inner| inner.check(who, what, action))
    }
}

impl PermissionEngine for ConfinedEngine {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        let named = OrgId::of_path(what.as_str());
        let current = current_org();
        if let (Some((named, _)), Some(current)) = (&named, &current) {
            if named != current {
                return Decision::deny(format!("{} is outside org {}", what.as_str(), current.0));
            }
        }
        let (org, relative) = match (named, current) {
            (Some((org, rest)), _) => (org, rest),
            (None, Some(org)) => (org, what.as_str()),
            (None, None) if self.require_org && !matches!(who, Principal::Service { .. }) => {
                return Decision::deny(format!("{} is in no org", what.as_str()));
            }
            (None, None) => return self.rest(who, what, action),
        };
        if matches!(who, Principal::Service { .. }) {
            return self.rest(who, what, action);
        }
        let Some(role) = self.members.role(who, &org) else {
            return Decision::deny(format!(
                "{} is not a member of org {}",
                who.describe(),
                org.0
            ));
        };
        if !self.roles.is_empty() {
            let relative = Resource::new(relative);
            let allowed = self
                .roles
                .get(&role.label())
                .is_some_and(|rules| rules.iter().any(|r| r.permits(&relative, action)));
            if !allowed {
                return Decision::deny(format!(
                    "a {} of org {} may not {} {}",
                    role.label(),
                    org.0,
                    action.as_str(),
                    relative.as_str()
                ));
            }
        }
        self.rest(who, what, action)
    }

    fn survey(&self, who: &Principal, prefix: &Resource) -> Vec<(Resource, Vec<Action>)> {
        self.inner
            .as_ref()
            .map(|inner| inner.survey(who, prefix))
            .unwrap_or_default()
    }
}
