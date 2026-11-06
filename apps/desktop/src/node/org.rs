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
    BoxIdentityFuture, IdentityResolver, PermissionEngine, Principal, Rule,
};

use super::service::node::NodeHttpClient;

/// How long a resolved session is trusted before the issuer is asked again.
const SESSION_TTL: Duration = Duration::from_secs(60);

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
    router: LayerRouter,
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
        let identity = MembersResolver::new(
            IssuerSessions::new(issuer.clone()),
            IssuerOrgs(issuer),
            members.clone(),
        );
        let gate = PermissionsGate::new(
            Arc::clone(&engine) as Arc<dyn PermissionEngine>,
            Arc::new(identity),
        )
        .permit(device_enrolment_service_descriptor(), &ENROL_PERMITS);
        let router = LayerRouter::new().with(
            device_enrolment_service_descriptor(),
            DeviceEnrolmentDispatcher::new(EnrolmentHost::new(admission.clone(), members.clone())),
        );
        Self {
            org,
            members,
            engine,
            gate: Arc::new(gate),
            router,
        }
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
                Ok(PermissionsGate::wrap_shared_with_bearer(
                    Arc::clone(&lane.gate),
                    lane.router.clone(),
                    head.bearer().map(str::to_owned),
                )
                .scoped(lane.org.id.clone()))
            }
        })
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

    async fn client<C>(&self) -> Result<C, String>
    where
        C: vox::FromVoxLane,
    {
        self.pool
            .client(&self.target)
            .await
            .map_err(|e| e.to_string())
    }

    fn evict(&self) {
        self.pool.evict(&self.target);
    }
}

/// An issuer session as the gate's caller: `Principal::User` of the
/// person it is (the issuer's `whoami`), cached for [`SESSION_TTL`];
/// anything else is anonymous.
pub struct IssuerSessions {
    issuer: IssuerLane,
    seen: Mutex<HashMap<String, (Principal, Instant)>>,
}

impl IssuerSessions {
    #[must_use]
    pub fn new(issuer: IssuerLane) -> Self {
        Self {
            issuer,
            seen: Mutex::new(HashMap::new()),
        }
    }

    fn cached(&self, token: &str) -> Option<Principal> {
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.get(token)
            .filter(|(_, at)| at.elapsed() < SESSION_TTL)
            .map(|(who, _)| who.clone())
    }
}

impl IdentityResolver for IssuerSessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        Box::pin(async move {
            let Some(token) = bearer_token.filter(|t| !t.is_empty()) else {
                return Principal::Anonymous;
            };
            if let Some(who) = self.cached(token) {
                return who;
            }
            let answer = match self.issuer.client::<auth_proto::AuthServiceClient>().await {
                Ok(auth) => auth
                    .whoami(token.to_owned())
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            let who = match answer {
                Ok(user) => Principal::User {
                    user_id: user.id.to_string(),
                },
                Err(why) => {
                    tracing::debug!(%why, "org lane: a session the issuer did not resolve");
                    if why.contains("connect") || why.contains("closed") {
                        self.issuer.evict();
                    }
                    return Principal::Anonymous;
                }
            };
            let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            seen.retain(|_, (_, at)| at.elapsed() < SESSION_TTL);
            seen.insert(token.to_owned(), (who.clone(), Instant::now()));
            who
        })
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
