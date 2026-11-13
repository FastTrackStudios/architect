//! The node in an issuer org: the org's enrolment lane, and enrolling
//! itself in an org another node hosts.
//!
//! - **Hosting** ([`Config::host_org`]). The node serves
//!   `architect_devices::server::EnrolmentHost` (`DeviceEnrolment`) at
//!   `/org/{slug}/vox` on its HTTP side. A caller presents an issuer
//!   session as its bearer ([`IssuerSessions`] resolves it, and
//!   `architect_orgs::server::MembersResolver` learns the person's orgs
//!   and role from the issuer). A member enrols a device there with a
//!   role at most their own, lists the org's enrolments and revokes
//!   them. An enrolment is a `Grant::Org` in the node's `Admission`: the
//!   device's iroh calls are let in, and it opens the vaults' notes when
//!   its role may write them ([`OrgLane::engine`],
//!   `crdt::p2p::NodeAccess::org_docs`). A revoke refuses its next call.
//! - **Enrolling** ([`Config::enrol_at`]). A node signed in as a member
//!   enrols its own endpoint at another node's lane, once ([`enrol_once`]):
//!   it learns that node's endpoint from its `/node/status` face, lets it
//!   in as the org's (`Grant::Org`), and syncs with it like any peer. A
//!   revoke is final: the node does not enrol itself again.
//!
//! - **Vaults for signed-in clients** ([`OrgLane::with_vaults`]). The same
//!   lane carries the vaults: `Notes` (list and read for any member,
//!   write for a member or above) and `DocSync` on the vaults' registry, so
//!   a web app signed in at the issuer edits a note live over
//!   `/org/{slug}/vox` with its session as the bearer. Which notes a person
//!   opens is the org's engine's call ([`OrgLane::opens`]): a viewer reads,
//!   a member writes.
//!
//! The node's vaults are the org's: one org per node.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use architect::host::{EngineHost, RequestHead};
use architect::http::axum::http::StatusCode;
use architect::layer::LayerRouter;
use architect::permissions_gate::PermissionsGate;
use architect_devices::server::{EnrolmentHost, project_members};
use architect_devices::{
    Admission, DeviceEnrolmentClient, DeviceEnrolmentDispatcher, ENROL_PERMITS, Enrolment, Grant,
    Peer, device_enrolment_service_descriptor,
};
use architect_orgs::server::{
    BoxOrgsFuture, ConfinedEngine, Directory, IdentityServer, Members, MembersResolver,
};
use architect_orgs::{OrgId, Role};
use architect_permissions::{
    Action, BoxIdentityFuture, IdentityResolver, MethodPermit, PermissionEngine, Principal,
    Resource, Rule, ServicePermits,
};
use uuid::Uuid;

use super::service::node::NodeHttpClient;

/// How long a resolved bearer is trusted before the issuer is asked again,
/// and how often an open live note checks its bearer still opens it
/// (`Config::lane_recheck_secs`; this is its default).
pub const RECHECK: Duration = Duration::from_secs(30);

/// How long an open live note outlives an issuer that cannot be reached.
///
/// Counted from the last check the issuer answered
/// (`Config::lane_issuer_grace_secs`; this is its default). Past it the
/// session ends. A new call is never admitted on an old answer.
pub const ISSUER_GRACE: Duration = Duration::from_secs(300);

/// What each role of the org may do on this node, org-relative: the
/// gate's rules for the enrolment lane and the documents' for the
/// devices enrolled in it.
#[must_use]
pub fn engine(members: Members) -> ConfinedEngine {
    let read = || vec![Rule::new("**", &["read"])];
    let write = || vec![Rule::new("**", &["read", "write"])];
    let admin = || vec![Rule::new("**", &["read", "write", "admin"])];
    ConfinedEngine::new(members)
        .role(&Role::Viewer, read())
        .role(&Role::Member, write())
        .role(&Role::Admin, admin())
        .role(&Role::Owner, admin())
}

/// `Notes` on the org lane: any member lists and reads, writing is a
/// write.
const NOTES_PERMITS: ServicePermits = ServicePermits {
    service: "Notes",
    methods: &[
        MethodPermit::new("list", "read", "vaults/**"),
        MethodPermit::new("read", "read", "vaults/**"),
        MethodPermit::new("write", "write", "vaults/**").audited(),
        MethodPermit::new("create", "write", "vaults/**").audited(),
    ],
};

/// `DocSync` on the org lane; which note is [`OrgLane::opens`]' call.
const SYNC_PERMITS: ServicePermits = ServicePermits {
    service: "DocSync",
    methods: &[MethodPermit::new("sync", "read", "docs/**")],
};

/// The org this node's vaults belong to, as the issuer names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeOrg {
    pub id: OrgId,
    pub slug: String,
}

/// The org's enrolment lane and the engine both lanes ask.
#[derive(Clone)]
pub struct OrgLane {
    pub org: NodeOrg,
    pub members: Members,
    pub engine: Arc<ConfinedEngine>,
    gate: Arc<PermissionsGate>,
    /// The gate's sessions resolver, which a live note's watch also asks
    /// (it tells a refusal from an issuer it cannot reach).
    sessions: Arc<IssuerSessions>,
    router: LayerRouter,
    issuer: IssuerLane,
    /// The vaults' live notes (`DocSync`), added per connection: each
    /// session watches that connection's bearer ([`VaultSync`]).
    vaults: Option<(crdt::DocRegistry, super::VaultCatalog)>,
    shares: bool,
    recheck: Duration,
    issuer_grace: Duration,
    /// Browser origins besides the node's own that may open the lane.
    origins: Arc<[String]>,
}

impl OrgLane {
    /// The lane for `org` over `admission`, its callers resolved by the
    /// issuer at `issuer` (`/vox`).
    #[must_use]
    pub fn new(org: NodeOrg, admission: &Admission, issuer: &str) -> Self {
        let members = Members::new();
        project_members(admission, &members);
        let engine = Arc::new(engine(members.clone()));
        let issuer = IssuerLane::new(issuer);
        let router = LayerRouter::new().with(
            device_enrolment_service_descriptor(),
            DeviceEnrolmentDispatcher::new(EnrolmentHost::new(admission.clone(), members.clone())),
        );
        let sessions = Arc::new(IssuerSessions::new(issuer.clone()).with_ttl(RECHECK));
        let gate = gate(&engine, &members, &issuer, &sessions, false, false, RECHECK);
        Self {
            org,
            members,
            engine,
            gate,
            sessions,
            router,
            issuer,
            vaults: None,
            shares: false,
            recheck: RECHECK,
            issuer_grace: ISSUER_GRACE,
            origins: Arc::from(Vec::new()),
        }
    }

    fn rebuild_gate(&mut self) {
        self.sessions = Arc::new(IssuerSessions::new(self.issuer.clone()).with_ttl(self.recheck));
        self.gate = gate(
            &self.engine,
            &self.members,
            &self.issuer,
            &self.sessions,
            self.vaults.is_some(),
            self.shares,
            self.recheck,
        );
    }

    /// Trust a resolved bearer for `every` (default [`RECHECK`]) and check
    /// an open live note's bearer as often: signed out, expired or no
    /// longer a member, its session ends within about twice `every`.
    #[must_use]
    pub fn with_recheck(mut self, every: Duration) -> Self {
        self.recheck = every.max(Duration::from_millis(100));
        self.rebuild_gate();
        self
    }

    /// Keep an open live note for `grace` (default [`ISSUER_GRACE`]) after
    /// the issuer last answered for its bearer, while the issuer cannot be
    /// reached; then end it. A refusal ends it at once, and a new call is
    /// refused while the issuer is unreachable.
    #[must_use]
    pub const fn with_issuer_grace(mut self, grace: Duration) -> Self {
        self.issuer_grace = grace;
        self
    }

    /// Serve the vaults' share links on the lane too
    /// (`architect_spaces::ShareLinks`): any member lists them, a member
    /// or above makes, changes and revokes one.
    #[must_use]
    pub fn with_shares(mut self, shares: super::shares::Shares) -> Self {
        self.router = self.router.with(
            architect_spaces::share_links_rpc_service_descriptor(),
            architect_spaces::ShareLinksDispatcher::new(super::shares::SharesLane(shares)),
        );
        self.shares = true;
        self.rebuild_gate();
        self
    }

    /// Serve the vaults on the lane too: `notes` (the node's `Notes`
    /// router) and `DocSync` over `registry`, whose authorizer must ask
    /// [`Self::opens`] for people.
    #[must_use]
    pub fn with_vaults(
        mut self,
        registry: crdt::DocRegistry,
        catalog: super::VaultCatalog,
        notes: LayerRouter,
    ) -> Self {
        self.router = self.router.merge_router(notes);
        self.vaults = Some((registry, catalog));
        self.rebuild_gate();
        self
    }

    /// Let browsers on `origins` (`https://host[:port]`) open the lane
    /// besides the node's own origin ([`origin_allowed`]).
    #[must_use]
    pub fn with_origins(mut self, origins: Vec<String>) -> Self {
        self.origins = Arc::from(origins);
        self
    }

    /// The lane's services for one connection: each live note watches the
    /// bearer its own call was admitted with ([`VaultSync`]).
    fn connection_router(&self) -> LayerRouter {
        let Some((registry, catalog)) = &self.vaults else {
            return self.router.clone();
        };
        self.router.clone().with(
            crdt::sync::doc_sync_service_descriptor(),
            crdt::sync::DocSyncDispatcher::new(VaultSync {
                registry: registry.clone(),
                catalog: catalog.clone(),
                lane: self.clone(),
            }),
        )
    }

    /// Whether a person may open note `doc_id` live: a member of the org
    /// who may `write` it (a viewer reads its file through `Notes`, not
    /// a live session, which writes).
    #[must_use]
    pub fn opens(&self, who: &Principal, doc_id: Uuid) -> bool {
        matches!(who, Principal::User { .. })
            && self
                .engine
                .check(
                    who,
                    &Resource::new(format!("org/{}/docs/{doc_id}", self.org.id.0)),
                    &Action::write(),
                )
                .allowed()
    }

    /// Serve the lane at `/org/{slug}/vox` on `host`; any other slug is
    /// not found.
    #[must_use]
    pub fn mount(&self, host: EngineHost) -> EngineHost {
        let lane = self.clone();
        host.vox_at_permissioned("/org/{slug}/vox", move |head: RequestHead| {
            let lane = lane.clone();
            async move {
                if head.param("slug") != Some(lane.org.slug.as_str()) {
                    return Err(StatusCode::NOT_FOUND);
                }
                if !origin_allowed(&head.headers, &lane.origins) {
                    return Err(StatusCode::FORBIDDEN);
                }
                Ok(PermissionsGate::wrap_shared_with_bearer(
                    Arc::clone(&lane.gate),
                    lane.connection_router(),
                    head.bearer().map(str::to_owned),
                )
                .scoped(lane.org.id.clone()))
            }
        })
    }
}

/// Whether a WebSocket upgrade with these `headers` may open the lane.
///
/// A browser always sends `Origin` on an upgrade: it must be the node's
/// own (the app is served on the same origin as the lane: its authority
/// is the request's `Host`, or the `X-Forwarded-Host` a proxy in front
/// set) or one of `extra`. An upgrade with no `Origin` is a native client
/// (another node, the desktop app, a test): it goes on to the bearer
/// check, which is what admits anyone. Defence in depth: the bearer rides
/// a subprotocol that only script on the page can add, not a cookie a
/// foreign page's request would carry.
#[must_use]
pub fn origin_allowed(headers: &architect::http::axum::http::HeaderMap, extra: &[String]) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let Some(origin) = header("origin") else {
        return true;
    };
    let origin = origin.trim_end_matches('/');
    if extra.iter().any(|o| o.eq_ignore_ascii_case(origin)) {
        return true;
    }
    let Some(authority) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    [header("host"), header("x-forwarded-host")]
        .into_iter()
        .flatten()
        .any(|host| host.eq_ignore_ascii_case(authority))
}

/// The lane's gate: the org's engine, callers resolved by the issuer;
/// with `vaults`, `Notes` and `DocSync` permitted too, with `shares`
/// `ShareLinks`.
fn gate(
    engine: &Arc<ConfinedEngine>,
    members: &Members,
    issuer: &IssuerLane,
    sessions: &Arc<IssuerSessions>,
    vaults: bool,
    shares: bool,
    recheck: Duration,
) -> Arc<PermissionsGate> {
    let identity = MembersResolver::new(
        SharedSessions(Arc::clone(sessions)),
        IssuerOrgs(issuer.clone()),
        members.clone(),
    )
    .with_ttl(recheck);
    let engine: Arc<dyn PermissionEngine> = engine.clone();
    let mut gate = PermissionsGate::new(engine, Arc::new(identity))
        .permit(device_enrolment_service_descriptor(), &ENROL_PERMITS);
    if vaults {
        gate = gate
            .permit(
                super::service::notes::notes_service_descriptor(),
                &NOTES_PERMITS,
            )
            .permit(crdt::sync::doc_sync_service_descriptor(), &SYNC_PERMITS);
    }
    if shares {
        gate = gate.permit(
            architect_spaces::share_links_rpc_service_descriptor(),
            &architect_spaces::SHARE_PERMITS,
        );
    }
    Arc::new(gate)
}

/// `DocSync` for a client that names a note by its id alone: the note is
/// registered with its vault first (as the peer lane's `resolve` does by
/// name), then the registry serves the session.
///
/// The gate admits a session once, when it attaches; the session is then
/// one call held open for as long as the note is. So it watches the
/// credential that call was admitted with
/// ([`architect::permissions_gate::caller_bearer`]: the call's own
/// `authorization`, else the connection's bearer), whichever way it came:
/// every [`OrgLane::with_recheck`] interval it is resolved again (the
/// issuer asked once the cache has run out), and once it no longer opens
/// the note (signed out, its grant revoked, expired, no longer a member)
/// the session ends. A session admitted with no credential ends at once:
/// an anonymous caller opens no note.
#[derive(Clone)]
struct VaultSync {
    registry: crdt::DocRegistry,
    catalog: super::VaultCatalog,
    lane: OrgLane,
}

/// Whether to keep a live note after one re-check of its bearer.
#[derive(Debug, PartialEq, Eq)]
enum Recheck {
    Keep,
    End,
}

impl OrgLane {
    /// Returns once `bearer` no longer opens `doc_id` (at once without
    /// one). An issuer that cannot be reached keeps the session until
    /// [`Self::with_issuer_grace`] after it last answered.
    async fn watch(&self, bearer: Option<&str>, doc_id: Uuid) {
        let Some(bearer) = bearer.filter(|b| !b.is_empty()) else {
            tracing::info!(%doc_id, "org lane: a live note with no credential; its session ends");
            return;
        };
        let identity = self.gate.identity_resolver();
        let mut answered = Instant::now();
        loop {
            tokio::time::sleep(self.recheck).await;
            let checked = self.sessions.check(bearer).await;
            let verdict = match checked {
                Checked::User(_) => {
                    // Membership too (the issuer's org list), then the note.
                    let who = identity.resolve(Some(bearer)).await;
                    if self.opens(&who, doc_id) {
                        answered = Instant::now();
                        Recheck::Keep
                    } else {
                        Recheck::End
                    }
                }
                other => after_failed_check(&other, answered.elapsed(), self.issuer_grace),
            };
            if verdict == Recheck::End {
                tracing::info!(%doc_id, "org lane: a live note's sign-in ended; its session ends");
                return;
            }
        }
    }
}

/// A re-check the issuer did not answer with a person: refused, the
/// session ends; unreachable, it is kept while the last answer is younger
/// than `grace`.
fn after_failed_check(checked: &Checked, since_answered: Duration, grace: Duration) -> Recheck {
    match checked {
        Checked::Unreachable if since_answered < grace => Recheck::Keep,
        _ => Recheck::End,
    }
}

impl crdt::sync::DocSync for VaultSync {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<crdt::sync::SyncDown>,
    ) -> Result<(), crdt::sync::SyncError> {
        // Read before the first await: the call's task-local.
        let bearer = architect::permissions_gate::caller_bearer();
        let held = self
            .catalog
            .notes()
            .into_iter()
            .find(|n| n.doc_id == doc_id);
        let at = held.ok_or(crdt::sync::SyncError::UnknownDoc)?;
        self.catalog.register(&crdt::peer::DocAt {
            doc_id,
            name: at.name,
        })?;
        let session = crdt::sync::DocSync::sync(&self.registry, doc_id, from, up, down);
        tokio::select! {
            ended = session => ended,
            () = self.lane.watch(bearer.as_deref(), doc_id) => Err(crdt::sync::SyncError::Internal(
                "the sign-in ended: sign in again".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Checked, Recheck, after_failed_check, origin_allowed};
    use architect::http::axum::http::HeaderMap;
    use std::time::Duration;

    /// N4: an issuer that cannot be reached keeps a live note for the
    /// grace after its last answer, then ends it; a refusal ends it at
    /// once, however recent the last answer.
    #[test]
    fn an_unreachable_issuer_keeps_a_live_note_for_the_grace_only() {
        let grace = Duration::from_secs(300);
        let at = |secs| Duration::from_secs(secs);
        assert_eq!(
            after_failed_check(&Checked::Unreachable, at(10), grace),
            Recheck::Keep
        );
        assert_eq!(
            after_failed_check(&Checked::Unreachable, at(299), grace),
            Recheck::Keep
        );
        assert_eq!(
            after_failed_check(&Checked::Unreachable, at(300), grace),
            Recheck::End
        );
        assert_eq!(
            after_failed_check(&Checked::Refused, at(0), grace),
            Recheck::End
        );
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    /// #628 L3: the lane's own origin and the listed ones open it, a
    /// native client (no `Origin`) goes on to the bearer check, any other
    /// page is refused.
    #[test]
    fn only_the_app_s_origin_opens_the_lane_from_a_browser() {
        let extra = ["http://127.0.0.1:7413".to_owned()];
        let host = ("host", "node.example.com");
        assert!(origin_allowed(&headers(&[host]), &extra));
        assert!(origin_allowed(
            &headers(&[host, ("origin", "https://node.example.com")]),
            &extra
        ));
        assert!(origin_allowed(
            &headers(&[
                ("host", "127.0.0.1:7412"),
                ("origin", "http://127.0.0.1:7413")
            ]),
            &extra
        ));
        assert!(origin_allowed(
            &headers(&[
                ("host", "10.0.0.5:8080"),
                ("x-forwarded-host", "node.example.com"),
                ("origin", "https://node.example.com"),
            ]),
            &extra
        ));
        for foreign in [
            "https://evil.example",
            "null",
            "https://node.example.com.evil.example",
        ] {
            assert!(
                !origin_allowed(&headers(&[host, ("origin", foreign)]), &extra),
                "{foreign}"
            );
        }
    }
}

/// The issuer's `/vox`, one connection kept and remade when it drops.
#[derive(Clone)]
pub struct IssuerLane {
    pool: Arc<architect::connect::Pool>,
    target: architect::connect::Endpoint,
}

impl IssuerLane {
    #[must_use]
    pub fn new(issuer: &str) -> Self {
        Self {
            pool: Arc::new(architect::connect::Pool::new()),
            target: architect::connect::Endpoint::url(super::vox_url(issuer)),
        }
    }

    pub(super) async fn client<C>(&self) -> Result<C, String>
    where
        C: vox::FromVoxLane,
    {
        self.pool
            .client(&self.target)
            .await
            .map_err(|e| e.to_string())
    }

    pub(super) fn evict(&self) {
        self.pool.evict(&self.target);
    }
}

/// An issuer session (or a first-party web app's access token) as the
/// gate's caller.
///
/// `Principal::User` of the person it is (the issuer's `whoami`), cached
/// for [`RECHECK`] (`with_ttl`); anything else is anonymous, including an
/// issuer that cannot be reached (a new call fails closed). A bearer
/// signed out at the issuer keeps working here until its cache entry runs
/// out.
pub struct IssuerSessions {
    issuer: IssuerLane,
    ttl: Duration,
    seen: Mutex<HashMap<String, (Principal, Instant)>>,
}

/// What the issuer said of a bearer.
#[derive(Debug)]
pub(crate) enum Checked {
    /// This person (answered now, or within the cache's lifetime).
    User(Principal),
    /// The issuer answered: not a sign-in it honours.
    Refused,
    /// No answer: the issuer could not be reached, or the call broke.
    Unreachable,
}

impl IssuerSessions {
    #[must_use]
    pub fn new(issuer: IssuerLane) -> Self {
        Self {
            issuer,
            ttl: RECHECK,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Trust a resolved bearer for `ttl`.
    #[must_use]
    pub const fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    fn cached(&self, token: &str) -> Option<Principal> {
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.get(token)
            .filter(|(_, at)| at.elapsed() < self.ttl)
            .map(|(who, _)| who.clone())
    }

    /// Asks the issuer who `token` is (or the cache, while fresh). The
    /// token is a credential: nothing here logs it.
    pub(crate) async fn check(&self, token: &str) -> Checked {
        if let Some(who) = self.cached(token) {
            return Checked::User(who);
        }
        let answer = match self.issuer.client::<auth_proto::AuthServiceClient>().await {
            Ok(auth) => auth.whoami(token.to_owned()).await.map_err(|e| {
                // A refusal is the issuer's answer; a broken or timed-out
                // call is no answer at all.
                let refused = matches!(
                    e,
                    vox::VoxError::User(_)
                        | vox::VoxError::InvalidPayload(_)
                        | vox::VoxError::UnknownMethod
                );
                (refused, e.to_string())
            }),
            Err(e) => Err((false, e)),
        };
        match answer {
            Ok(user) => {
                let who = Principal::User {
                    user_id: user.id.to_string(),
                };
                {
                    let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
                    seen.retain(|_, (_, at)| at.elapsed() < self.ttl);
                    seen.insert(token.to_owned(), (who.clone(), Instant::now()));
                }
                Checked::User(who)
            }
            Err((true, why)) => {
                tracing::debug!(%why, "org lane: a bearer the issuer refused");
                Checked::Refused
            }
            Err((false, why)) => {
                tracing::debug!(%why, "org lane: the issuer did not answer");
                self.issuer.evict();
                Checked::Unreachable
            }
        }
    }
}

impl IdentityResolver for IssuerSessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        Box::pin(async move {
            let Some(token) = bearer_token.filter(|t| !t.is_empty()) else {
                return Principal::Anonymous;
            };
            match self.check(token).await {
                Checked::User(who) => who,
                Checked::Refused | Checked::Unreachable => Principal::Anonymous,
            }
        })
    }
}

/// The lane's one [`IssuerSessions`] as the gate's resolver, shared with
/// its live notes' watches.
struct SharedSessions(Arc<IssuerSessions>);

impl IdentityResolver for SharedSessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        self.0.resolve(bearer_token)
    }
}

/// The issuer's organizations as the gate's [`Directory`].
struct IssuerOrgs(IssuerLane);

impl Directory for IssuerOrgs {
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a> {
        Box::pin(async move {
            let client = self
                .0
                .client::<auth_proto::OrganizationServiceClient>()
                .await?;
            IdentityServer(client).orgs(token).await
        })
    }
}

/// The issuer org `slug` names, for the session `token`.
///
/// # Errors
///
/// The issuer did not answer, or the session is in no org of that slug.
pub async fn resolve(issuer: &str, token: &str, slug: &str) -> Result<NodeOrg, String> {
    let lane = IssuerLane::new(issuer);
    let client = lane
        .client::<auth_proto::OrganizationServiceClient>()
        .await?;
    let orgs = IdentityServer(client).orgs(token).await?;
    orgs.into_iter()
        .find(|o| o.slug == slug)
        .map(|o| NodeOrg {
            id: o.id,
            slug: o.slug,
        })
        .ok_or_else(|| format!("the session is in no org `{slug}`"))
}

/// The org a node resolved once, kept in its data dir so it starts with
/// the issuer down.
#[must_use]
pub fn remembered(data: &Path, slug: &str) -> Option<NodeOrg> {
    let text = std::fs::read_to_string(org_file(data)).ok()?;
    let (kept, id) = text.trim().split_once('=')?;
    (kept == slug).then(|| NodeOrg {
        id: OrgId::new(id),
        slug: slug.to_owned(),
    })
}

/// Keep `org` for [`remembered`].
pub fn remember(data: &Path, org: &NodeOrg) {
    if let Err(error) = std::fs::write(org_file(data), format!("{}={}", org.slug, org.id.0)) {
        tracing::warn!(%error, "the org is not kept: the next start asks the issuer");
    }
}

fn org_file(data: &Path) -> PathBuf {
    data.join("org")
}

/// Where a node enrols itself, and as what.
#[derive(Debug, Clone)]
pub struct EnrolAt {
    /// The hosting node's HTTP origin (`https://node.example`).
    pub at: String,
    pub org: NodeOrg,
    pub role: Role,
    /// This node's endpoint id and name.
    pub endpoint: String,
    pub name: String,
}

/// Enrol this node at the hosting node, as a `server` device with
/// `enrol.role`, using `token`; then let that node in as the org's.
///
/// # Errors
///
/// The hosting node or its lane did not answer, or refused.
pub async fn enrol_once(
    enrol: &EnrolAt,
    token: &str,
    admission: &Admission,
) -> Result<Peer, String> {
    let origin = enrol.at.trim_end_matches('/');
    let host = NodeHttpClient::at(origin.to_owned())
        .status()
        .await
        .map_err(|e| format!("its /node/status: {e}"))?;
    let lane = architect::connect::Endpoint::url(format!(
        "{}/org/{}/vox",
        super::vox_url(origin).trim_end_matches("/vox"),
        enrol.org.slug
    ))
    .with_bearer(Some(token.to_owned()));
    let enrolment: DeviceEnrolmentClient = architect::connect::Pool::new()
        .client(&lane)
        .await
        .map_err(|e| format!("its org lane: {e}"))?;
    let enrolled = enrolment
        .enrol(Enrolment {
            endpoint: enrol.endpoint.clone(),
            name: enrol.name.clone(),
            platform: "server".to_owned(),
            role: enrol.role.clone(),
        })
        .await
        .map_err(|e| e.to_string())?;
    admission.admit(
        Peer::new(&host.endpoint, host.name, "server").granted(Grant::Org {
            org: enrol.org.id.clone(),
            role: Role::Member,
            by: format!("enrolled-at:{origin}"),
        }),
    );
    Ok(enrolled)
}

/// The marker a node leaves once it enrolled at `at`: it never enrols
/// there again by itself, so a revoke holds.
#[must_use]
pub fn enrolled_marker(data: &Path) -> PathBuf {
    data.join("enrolled")
}
