//! A node's documents, served peer to peer: to the owner's other devices
//! and to whoever holds a link, with no server between them.
//!
//! [`crate::peer`] moves documents between nodes and [`crate::sync`]
//! keeps a replica live against a host; this module decides **who** a
//! node serves them to, and mounts both lanes for one caller:
//!
//! - **[`Admission`]** (`architect_devices`): the devices this node lets
//!   in — the owner's other devices (the account pass keeps them
//!   current), and devices paired with it for some documents
//!   (`Grant::Paired`). A device revoked from it is refused on its next
//!   call, and its live sessions end.
//! - **[`Shares`]**: the live links this node has handed out
//!   ([`DocLink`]). Each opens one document to whoever holds it, until
//!   revoked; revoking one also ends the sessions running through it.
//! - **[`NodeAccess`]**: both, as the answers the gate and the registry
//!   ask for: who the caller is ([`NodeAccess::caller`]), whether they
//!   may call at all ([`NodeAccess`] is the gate's
//!   [`PermissionEngine`]), and which documents they may open
//!   ([`NodeAccess::admits`], for [`DocRegistry::with_authorizer`]).
//! - **[`node_router`]**: the router one caller gets: `DocSync` (live
//!   editing) and `DocPeer` (sweeps between devices) over the node's
//!   registry, behind a gate that resolves the caller from the endpoint
//!   id the connection proved. Serve it with `architect_devices::serve`
//!   (`iroh_link::serve_peers_until`), which makes one per connection
//!   and closes it when its device is revoked.
//!
//! ## Who is calling
//!
//! The endpoint id comes from the transport, never from the caller: an
//! iroh connection proves the peer holds that key. A link's key is
//! different, a credential presented on each call (`authorization:
//! Bearer link:<key>`, [`LinkKey`](crate::link::LinkKey)); a caller
//! presenting a live key is that link's `Principal::Guest`, everyone else
//! is `Principal::Host` of the endpoint they proved. Then:
//!
//! | Caller | May call | May open |
//! |---|---|---|
//! | one of the owner's devices (`Host`, `Grant::Account`) | `DocSync`, `DocPeer` | whatever the registry's authorizer allows ([`NodeAccess::admits`]: every document) |
//! | a device enrolled in the node's org (`Host`, `Grant::Org`, [`NodeAccess::org_docs`]) | `DocSync`, `DocPeer` when its role may write `docs/**` | what the org's engine gives its role there (`org/<org>/docs/<id>`) |
//! | a paired device (`Host`, `Grant::Paired`) | `DocSync` | the documents its scopes name (`docs/<id>`) |
//! | a link's guest (`Guest`, a live key) | `DocSync` | the link's document only |
//! | any other endpoint | nothing | nothing |

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use architect::layer::LayerRouter;
use architect::permissions_gate::{PermissionedRouter, PermissionsGate};
pub use architect_devices::{Admission, Pairings};
use architect_devices::{
    DevicePairingDispatcher, PAIR_PERMITS, PairingHost, device_pairing_service_descriptor,
};
use architect_permissions::{
    Action, BoxIdentityFuture, Decision, IdentityResolver, MethodPermit, PermissionEngine,
    Principal, Resource, ServicePermits,
};
use uuid::Uuid;

use crate::link::{DocLink, LINK_BEARER_PREFIX};
use crate::peer::{DocCatalog, DocPeerDispatcher, DocPeerHost, doc_peer_service_descriptor};
use crate::registry::DocRegistry;
use crate::sync::{DocSync, DocSyncDispatcher, SyncDown, SyncError, doc_sync_service_descriptor};

// ── Shares ─────────────────────────────────────────────────────────────

/// One live link.
#[derive(Debug, Clone)]
struct Share {
    doc_id: Uuid,
    /// Names the link in `Principal::Guest` and audit lines; the key
    /// itself is never logged.
    link_id: String,
}

/// The links this node has handed out, by key.
///
/// Cheap to clone; every clone sees the same links.
#[derive(Debug, Clone)]
pub struct Shares {
    links: Arc<RwLock<HashMap<String, Share>>>,
    /// Bumped on every revoke, so a guest's running session can check
    /// whether its link is still live.
    revoked: Arc<tokio::sync::watch::Sender<u64>>,
}

impl Default for Shares {
    fn default() -> Self {
        Self {
            links: Arc::default(),
            revoked: Arc::new(tokio::sync::watch::Sender::new(0)),
        }
    }
}

impl Shares {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open `doc_id` to whoever holds the link this returns: a fresh key
    /// on this node (`endpoint`, its iroh endpoint id). Each call is a
    /// separate link, revoked on its own.
    #[must_use]
    pub fn share(&self, endpoint: &str, doc_id: Uuid) -> DocLink {
        let key = Uuid::new_v4().simple().to_string();
        let link_id = Uuid::new_v4().to_string();
        self.write().insert(key.clone(), Share { doc_id, link_id });
        DocLink {
            endpoint: endpoint.to_ascii_lowercase(),
            doc_id,
            key,
        }
    }

    /// Close one link: its key is refused from the next call on, and a
    /// guest editing through it is cut off.
    pub fn revoke(&self, link: &DocLink) {
        self.write().remove(&link.key);
        self.revoked.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Close every link to `doc_id`.
    pub fn revoke_doc(&self, doc_id: Uuid) {
        self.write().retain(|_, share| share.doc_id != doc_id);
        self.revoked.send_modify(|n| *n = n.wrapping_add(1));
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Share>> {
        self.links.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Share>> {
        self.links.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Resolves once `link_id` is no longer live.
    async fn until_revoked(&self, link_id: &str) {
        let mut revoked = self.revoked.subscribe();
        while self.live(link_id) {
            if revoked.changed().await.is_err() {
                // The sender lives as long as `self`: unreachable, but
                // never end a session for it.
                std::future::pending::<()>().await;
            }
        }
    }

    /// The guest a live `key` makes its holder.
    fn guest(&self, key: &str) -> Option<Principal> {
        self.read().get(key).map(|share| Principal::Guest {
            link_id: share.link_id.clone(),
            display: None,
        })
    }

    /// Whether the link `link_id` is live and opens `doc_id`.
    fn opens(&self, link_id: &str, doc_id: Uuid) -> bool {
        self.read()
            .values()
            .any(|share| share.link_id == link_id && share.doc_id == doc_id)
    }

    /// Whether the link `link_id` is live at all.
    fn live(&self, link_id: &str) -> bool {
        self.read().values().any(|share| share.link_id == link_id)
    }
}

// ── Access ─────────────────────────────────────────────────────────────

/// Who a node serves: the devices its [`Admission`] lets in, its
/// [`Shares`], and, when it takes pairings, its open offers.
#[derive(Debug, Clone, Default)]
pub struct NodeAccess {
    pub devices: Admission,
    pub shares: Shares,
    /// The node's pairing offers (`architect_devices::Pairings`, over the
    /// same `devices`): when set, [`node_router`] also serves
    /// `DevicePairing`, which any endpoint may call.
    pub pairings: Option<Pairings>,
    /// The org whose documents these are, for its enrolled devices
    /// ([`Self::org_docs`]).
    pub org: Option<OrgDocs>,
}

/// The org a node's documents are, and the engine that says what each
/// role there may do ([`NodeAccess::org_docs`]).
#[derive(Clone)]
pub struct OrgDocs {
    /// The org's id (`architect_orgs::OrgId`'s string).
    pub org: String,
    /// The org's engine: `architect_orgs::server::ConfinedEngine`, with
    /// the org's enrolled devices as `Principal::Host` members
    /// (`architect_devices::server::project_members`).
    pub engine: Arc<dyn PermissionEngine>,
}

impl std::fmt::Debug for OrgDocs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrgDocs")
            .field("org", &self.org)
            .finish_non_exhaustive()
    }
}

impl NodeAccess {
    #[must_use]
    pub const fn new(devices: Admission, shares: Shares) -> Self {
        Self {
            devices,
            shares,
            pairings: None,
            org: None,
        }
    }

    /// The node's documents are org `org`'s: a device enrolled there
    /// (`Grant::Org`) opens a document when `engine` lets it `write`
    /// `org/<org>/docs/<id>`, and sweeps (`DocPeer`) when it may write
    /// `org/<org>/docs/**`. With the org's `ConfinedEngine` that is what
    /// its role there may do, as for a person with that role; revoking
    /// its enrolment refuses its next call.
    #[must_use]
    pub fn org_docs(mut self, org: impl Into<String>, engine: Arc<dyn PermissionEngine>) -> Self {
        self.org = Some(OrgDocs {
            org: org.into(),
            engine,
        });
        self
    }

    /// Whether `endpoint`, enrolled in the node's org, may `action` the
    /// org-relative `what`.
    fn org_allows(&self, endpoint: &str, what: &str, action: &Action) -> bool {
        let Some(docs) = &self.org else {
            return false;
        };
        let enrolled = self
            .devices
            .get(endpoint)
            .is_some_and(|peer| peer.orgs().iter().any(|(org, _)| org.0 == docs.org));
        enrolled
            && docs
                .engine
                .check(
                    &Principal::Host {
                        endpoint: endpoint.to_owned(),
                    },
                    &Resource::new(format!("org/{}/{what}", docs.org)),
                    action,
                )
                .allowed()
    }

    /// Whether `endpoint` may sweep this node (`DocPeer`, which lists
    /// every document): one of the owner's devices, or one enrolled in
    /// the node's org whose role may write all its documents.
    #[must_use]
    pub fn sweeps(&self, endpoint: &str) -> bool {
        self.devices.is_account(endpoint) || self.org_allows(endpoint, "docs/**", &Action::write())
    }

    /// Take pairings: a device that redeems one of `pairings`' offers
    /// over its connection is let into [`Self::devices`] with the offer's
    /// scopes. Make `pairings` over the same `Admission`.
    #[must_use]
    pub fn pairing(mut self, pairings: Pairings) -> Self {
        self.pairings = Some(pairings);
        self
    }

    /// Who a call is from: the link's guest when it presents a live
    /// link key (`bearer`, without `Bearer `), otherwise the endpoint the
    /// connection proved (`remote`).
    #[must_use]
    pub fn caller(&self, remote: &str, bearer: Option<&str>) -> Principal {
        bearer
            .and_then(|b| b.strip_prefix(LINK_BEARER_PREFIX))
            .and_then(|key| self.shares.guest(key))
            .unwrap_or_else(|| Principal::Host {
                endpoint: remote.to_owned(),
            })
    }

    /// Whether `who` may open `doc_id`: one of the owner's devices any
    /// document, a paired device the documents its scopes name, a link's
    /// guest the link's. Hand it to [`DocRegistry::with_authorizer`],
    /// beside the app's own rules.
    #[must_use]
    pub fn admits(&self, who: Option<&Principal>, doc_id: Uuid) -> bool {
        match who {
            Some(Principal::Host { endpoint }) => {
                self.devices.allows(
                    endpoint,
                    &architect_devices::doc_resource(doc_id),
                    &Action::write(),
                ) || self.org_allows(endpoint, &format!("docs/{doc_id}"), &Action::write())
            }
            Some(Principal::Guest { link_id, .. }) => self.shares.opens(link_id, doc_id),
            _ => false,
        }
    }
}

/// The gate's engine: a device the node lets in or a live link's guest
/// may call (which documents is the registry's authorizer's call);
/// nobody else may call anything.
impl PermissionEngine for NodeAccess {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        match who {
            Principal::Host { endpoint } if self.devices.admits(endpoint) => Decision::Allow,
            // Asking to pair: `Admission` lets any endpoint.
            host @ Principal::Host { .. } if self.pairings.is_some() => {
                self.devices.check(host, what, action)
            }
            Principal::Guest { link_id, .. } if self.shares.live(link_id) => Decision::Allow,
            Principal::Host { .. } => Decision::deny("not a device this node lets in"),
            Principal::Guest { .. } => Decision::deny("the link was revoked"),
            other => Decision::deny(format!("{} is not a peer", other.describe())),
        }
    }
}

/// The gate's identity resolver for one connection: [`NodeAccess::caller`]
/// with the connection's proved endpoint id.
struct ConnectionCaller {
    access: NodeAccess,
    remote: String,
}

impl IdentityResolver for ConnectionCaller {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = self.access.caller(&self.remote, bearer_token);
        Box::pin(async move { who })
    }
}

// ── The router ─────────────────────────────────────────────────────────

/// `DocSync` over the node's registry, ending a guest's session when its
/// link is revoked and a device's when it is revoked.
#[derive(Clone)]
struct NodeSync {
    registry: DocRegistry,
    shares: Shares,
    devices: Admission,
}

impl DocSync for NodeSync {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError> {
        let session = self.registry.sync(doc_id, from, up, down);
        match architect::permissions_gate::caller() {
            Some(Principal::Guest { link_id, .. }) => {
                tokio::select! {
                    res = session => res,
                    () = self.shares.until_revoked(&link_id) => {
                        tracing::debug!(?doc_id, "p2p: a guest's link was revoked mid-session");
                        Err(SyncError::UnknownDoc)
                    }
                }
            }
            Some(Principal::Host { endpoint }) => {
                tokio::select! {
                    res = session => res,
                    () = self.devices.ends(&endpoint) => {
                        tracing::debug!(?doc_id, "p2p: a device was revoked mid-session");
                        Err(SyncError::UnknownDoc)
                    }
                }
            }
            _ => session.await,
        }
    }
}

/// `DocSync` for [`node_router`]'s gate; which document is the
/// registry's authorizer's call.
const SYNC_PERMITS: ServicePermits = ServicePermits {
    service: "DocSync",
    methods: &[MethodPermit::new("sync", "write", "docs/**")],
};

/// `DocPeer` for [`node_router`]'s gate; [`DocPeerHost`] refuses
/// everyone but the owner's own devices on top.
const PEER_PERMITS: ServicePermits = ServicePermits {
    service: "DocPeer",
    methods: &[
        MethodPermit::new("list", "read", "docs/**"),
        MethodPermit::new("sync", "write", "docs/**"),
    ],
};

/// The router one connection from `remote` is served.
///
/// `remote` is the endpoint id the connection proved. The router holds
/// `DocSync` over `registry` for live editing, `DocPeer` over `registry`
/// and `catalog` for sweeps (the owner's devices only: a sweep lists
/// every document), and `DevicePairing` when `access` takes pairings,
/// behind a gate that answers with `access`. Anything else on the
/// connection is refused.
///
/// `registry` must consult [`NodeAccess::admits`] in its authorizer (the
/// gate decides who may call; the registry, which documents).
///
/// ```rust,ignore
/// let serving = endpoint.clone();
/// tokio::spawn(async move {
///     architect_devices::serve(&serving, access.devices.clone(), move |remote| {
///         node_router(registry.clone(), OpenDocs(registry.clone()), access.clone(), &remote.to_string())
///     })
///     .await;
/// });
/// ```
#[must_use]
pub fn node_router<C: DocCatalog + Clone>(
    registry: DocRegistry,
    catalog: C,
    access: NodeAccess,
    remote: &str,
) -> PermissionedRouter {
    let sweepers = access.clone();
    let host = DocPeerHost::new(registry.clone(), catalog, move |endpoint| {
        sweepers.sweeps(endpoint)
    });
    let router = LayerRouter::new()
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(NodeSync {
                registry,
                shares: access.shares.clone(),
                devices: access.devices.clone(),
            }),
        )
        .with(doc_peer_service_descriptor(), DocPeerDispatcher::new(host));
    let router = match access.pairings.clone() {
        Some(pairings) => router.with(
            device_pairing_service_descriptor(),
            DevicePairingDispatcher::new(PairingHost(pairings)),
        ),
        None => router,
    };
    let identity = ConnectionCaller {
        access: access.clone(),
        remote: remote.to_ascii_lowercase(),
    };
    PermissionsGate::new(Arc::new(access), Arc::new(identity))
        .permit(doc_sync_service_descriptor(), &SYNC_PERMITS)
        .permit(doc_peer_service_descriptor(), &PEER_PERMITS)
        .permit(device_pairing_service_descriptor(), &PAIR_PERMITS)
        .wrap(router)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: &str = "aa";
    const STRANGER: &str = "bb";

    fn access() -> NodeAccess {
        let devices = Admission::new();
        devices.set_account([architect_devices::Peer::new(DEVICE, "phone", "ios")]);
        NodeAccess::new(devices, Shares::new())
    }

    #[test]
    fn a_device_opens_every_document_and_a_stranger_none() {
        let access = access();
        let doc = Uuid::from_u128(1);
        let device = access.caller(DEVICE, None);
        let stranger = access.caller(STRANGER, None);
        assert!(access.admits(Some(&device), doc));
        assert!(!access.admits(Some(&stranger), doc));
        assert!(!access.admits(None, doc));
        let any = Resource::new("docs/**");
        assert!(access.check(&device, &any, &Action::write()).allowed());
        assert!(!access.check(&stranger, &any, &Action::write()).allowed());

        access.devices.revoke(DEVICE);
        assert!(!access.admits(Some(&device), doc));
    }

    #[test]
    fn a_paired_device_opens_the_documents_its_scopes_name() {
        let access = access();
        let (doc, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        access.devices.admit(
            architect_devices::Peer::new(STRANGER, "ipad", "ios").granted(
                architect_devices::Grant::Paired {
                    scopes: vec![architect_devices::Scope::doc(doc, &["write"])],
                    by: "laptop".into(),
                },
            ),
        );
        let paired = access.caller(STRANGER, None);
        assert!(access.admits(Some(&paired), doc));
        assert!(!access.admits(Some(&paired), other));
        let any = Resource::new("docs/**");
        assert!(access.check(&paired, &any, &Action::write()).allowed());
        assert!(!access.devices.is_account(STRANGER), "no sweeps");
    }

    #[test]
    fn an_org_device_opens_what_its_role_may_write_until_revoked() {
        use architect_orgs::server::{ConfinedEngine, Members};
        use architect_orgs::{OrgId, Role};
        let members = Members::new();
        let engine = ConfinedEngine::new(members.clone())
            .role(
                &Role::Viewer,
                vec![architect_permissions::Rule::new("**", &["read"])],
            )
            .role(
                &Role::Member,
                vec![architect_permissions::Rule::new("**", &["read", "write"])],
            );
        let access = access().org_docs("acme", Arc::new(engine));
        architect_devices::server::project_members(&access.devices, &members);
        let org = |role| architect_devices::Grant::Org {
            org: OrgId::new("acme"),
            role,
            by: "user:ada".into(),
        };
        let doc = Uuid::from_u128(1);
        access.devices.admit(
            architect_devices::Peer::new(STRANGER, "server", "server").granted(org(Role::Member)),
        );
        let device = access.caller(STRANGER, None);
        assert!(access.admits(Some(&device), doc));
        assert!(access.sweeps(STRANGER));

        access.devices.admit(
            architect_devices::Peer::new(STRANGER, "server", "server").granted(org(Role::Viewer)),
        );
        assert!(
            !access.admits(Some(&device), doc),
            "a viewer does not write"
        );
        assert!(!access.sweeps(STRANGER));

        access.devices.revoke_org(STRANGER, &OrgId::new("acme"));
        assert!(!access.admits(Some(&device), doc));
        assert!(!access.devices.admits(STRANGER));
    }

    #[test]
    fn a_link_opens_its_document_until_revoked() {
        let access = access();
        let (doc, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let link = access.shares.share(&"cd".repeat(32), doc);
        let guest = access.caller(STRANGER, Some(&link.bearer()));
        assert!(matches!(guest, Principal::Guest { .. }));
        assert!(access.admits(Some(&guest), doc));
        assert!(!access.admits(Some(&guest), other));

        // A key nobody minted is no key: the caller is just its endpoint.
        let forged = access.caller(STRANGER, Some("link:forged"));
        assert_eq!(
            forged,
            Principal::Host {
                endpoint: STRANGER.into()
            }
        );

        access.shares.revoke(&link);
        assert!(!access.admits(Some(&guest), doc));
        let any = Resource::new("docs/**");
        assert!(!access.check(&guest, &any, &Action::write()).allowed());
    }
}
