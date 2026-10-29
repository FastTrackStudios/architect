//! Documents between nodes: pairwise exchange, no hub required.
//!
//! [`crate::sync`] is hub-and-spoke: replicas attach to one canonical
//! doc. That is half of local-first. The other half (ADR 0006, "every
//! node is a peer") is two nodes that each hold a [`DocRegistry`]
//! moving a document between themselves, with the always-on host
//! stopped or never involved. This module is that half, lifted from
//! Task's `vault-doc::peer` + `apps/server/src/doc_peer.rs` with the
//! vault taken out.
//!
//! - **[`DocPeer`]**: the lane a node serves to the peers it admits.
//!   [`DocPeer::list`] says which documents the node holds, each with an
//!   opaque digest the puller compares to skip what already matches;
//!   [`DocPeer::sync`] is the ordinary `DocSync` session, addressed by a
//!   [`DocAt`] so the serving node can resolve (and register) a document
//!   it has never been asked for. [`DocPeerHost`] serves it over a
//!   [`DocRegistry`] and a [`DocCatalog`].
//! - **[`exchange`]**: what the pulling node runs per document: one
//!   stateless pipe between its own registry and the peer's, through an
//!   in-memory replica. Local history goes up to the peer, the peer's
//!   comes down to the local registry, each through a normal client
//!   session, so the registry on each side persists, relays to whoever
//!   has the document open, and applies its own access rules exactly as
//!   for a device's editor. Nothing about the pipe is remembered.
//! - **[`pull`]**: one sweep against one peer: list, skip matching
//!   digests, exchange the rest, count what happened ([`PullReport`]).
//!
//! Both nodes sweep each other, so either pull alone converges two
//! documents; a pull carries both directions anyway, which is what lets
//! an edit reach a peer that cannot dial back (behind a NAT, asleep).
//! Relayed through a third node, an edit reaches a node it was never
//! directly exchanged with: A→B then B→C converges C with A while A is
//! offline (`tests/peer.rs`).
//!
//! ## Who may exchange
//!
//! Two checks, both against the principal the router's
//! [`PermissionsGate`](architect::permissions_gate::PermissionsGate)
//! resolved for the call ([`architect::permissions_gate::caller`]):
//!
//! 1. **The peer**: [`DocPeerHost`] serves only `Principal::Host`
//!    callers its `admit_peer` check accepts (an org's own devices, by
//!    iroh endpoint id). Everyone else, a signed-in user included, is
//!    refused on both `list` and `sync`; the lane is for nodes.
//! 2. **The document**: [`DocPeer::sync`] attaches through the
//!    registry's [`DocRegistry::with_authorizer`], which sees the same
//!    caller and decides per document.
//!
//! A history that shares nothing with the served document (two nodes
//! that each created "the same" document separately) is refused with
//! [`FOREIGN_LINEAGE`] instead of merged, which would put every line in
//! twice ([`is_foreign_lineage`]).
//!
//! ## The local lane
//!
//! [`exchange`] talks to this node's own registry as a client. Use
//! [`SelfLane`]: an in-process lane behind a gate that presents a fixed
//! principal, so the registry's authorizer sees *who* is pulling rather
//! than `None`.

use std::collections::HashMap;
use std::future::Future;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::Duration;

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::{PermissionsGate, StaticPrincipal};
use architect_permissions::{AllowAll, MethodPermit, Principal, ServicePermits};
use loro::{LoroDoc, VersionVector};
use uuid::Uuid;

use crate::CrdtDoc;
use crate::registry::DocRegistry;
use crate::sync::{DocSync, DocSyncClient, SyncDown, SyncError};

/// Prefix of the [`SyncError::BadVersion`] message a node answers a
/// foreign-lineage attach with.
pub const FOREIGN_LINEAGE: &str = "foreign-lineage";

/// A replica's history and the served document's share no peer: they
/// are two documents that happen to have one id. Either side being
/// empty is never foreign (a fresh document on one node).
#[must_use]
pub fn is_foreign_lineage(client: &VersionVector, server: &VersionVector) -> bool {
    let nonzero = |vv: &VersionVector| vv.iter().any(|(_, c)| *c > 0);
    if !nonzero(client) || !nonzero(server) {
        return false;
    }
    !client
        .iter()
        .any(|(peer, c)| *c > 0 && server.get(peer).is_some_and(|s| *s > 0))
}

/// Refuse an attach whose history shares nothing with the served doc.
///
/// `from` is the caller's encoded version vector; the served doc is
/// `registry`'s document of `doc_id`. An empty or
/// undecodable `from`, or a document the registry will not open, passes:
/// the registry's own attach reports those.
pub async fn check_lineage(
    registry: &DocRegistry,
    doc_id: Uuid,
    from: &[u8],
) -> Result<(), SyncError> {
    if from.is_empty() {
        return Ok(());
    }
    let Ok(client) = VersionVector::decode(from) else {
        return Ok(());
    };
    let Ok(doc) = registry.doc(doc_id).await else {
        return Ok(());
    };
    if is_foreign_lineage(&client, &doc.loro().oplog_vv()) {
        return Err(SyncError::BadVersion(format!(
            "{FOREIGN_LINEAGE}: this copy of {doc_id} is not a version of this node's"
        )));
    }
    Ok(())
}

// ── The wire ───────────────────────────────────────────────────────────

/// One document a node holds.
///
/// `name` is the app's own address for it
/// (a vault path, a project key) from which both nodes derive `doc_id`
/// (UUID v5); `digest` is whatever cheap fingerprint the app keeps (a
/// file's sha, [`vv_digest`]): equal non-empty digests are skipped.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct HeldDoc {
    pub doc_id: Uuid,
    pub name: String,
    pub digest: String,
}

/// Which document a [`DocPeer::sync`] is for.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct DocAt {
    pub doc_id: Uuid,
    pub name: String,
}

/// The document lane a node serves to the nodes it admits.
#[vox::service]
pub trait DocPeer {
    /// Every document this node holds and will exchange.
    async fn list(&self) -> Result<Vec<HeldDoc>, SyncError>;

    /// The `DocSync` session for the document `at` names. Refused when
    /// the caller's history is a different document of the same id
    /// ([`FOREIGN_LINEAGE`]).
    async fn sync(
        &self,
        at: DocAt,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError>;
}

// ── Serving ────────────────────────────────────────────────────────────

/// What a node holds, for [`DocPeerHost`]: the app's index of documents.
pub trait DocCatalog: Send + Sync + 'static {
    /// Every document this node will exchange.
    fn held(&self) -> impl Future<Output = Result<Vec<HeldDoc>, SyncError>> + Send;

    /// The registry id for `at`, registering it if the app keeps an
    /// index (a document this node does not hold yet; the session then
    /// makes it). Refuse an `at` whose `doc_id` is not the one its
    /// `name` derives to.
    fn resolve(&self, at: &DocAt) -> impl Future<Output = Result<Uuid, SyncError>> + Send;
}

/// A [`DocCatalog`] over whatever a registry has open: each open doc,
/// named by its id, digested by [`vv_digest`]; `resolve` accepts any id.
/// Enough for tests and for apps whose doc ids are the address.
#[derive(Clone)]
pub struct OpenDocs(pub DocRegistry);

impl DocCatalog for OpenDocs {
    async fn held(&self) -> Result<Vec<HeldDoc>, SyncError> {
        let mut out = Vec::new();
        for doc_id in self.0.open_docs().await {
            let doc = self.0.doc(doc_id).await?;
            out.push(HeldDoc {
                doc_id,
                name: doc_id.to_string(),
                digest: vv_digest(&doc.loro().oplog_vv()),
            });
        }
        Ok(out)
    }

    async fn resolve(&self, at: &DocAt) -> Result<Uuid, SyncError> {
        Ok(at.doc_id)
    }
}

/// A stable text form of a version vector (`peer:counter`, sorted), for
/// [`HeldDoc::digest`]: two nodes holding the same history print the
/// same digest.
#[must_use]
pub fn vv_digest(vv: &VersionVector) -> String {
    let mut parts: Vec<(u64, i32)> = vv.iter().map(|(p, c)| (*p, *c)).collect();
    parts.sort_unstable();
    parts
        .iter()
        .map(|(p, c)| format!("{p:x}:{c}"))
        .collect::<Vec<_>>()
        .join(",")
}

type PeerCheck = dyn Fn(&str) -> bool + Send + Sync;

/// The [`DocPeer`] lane over a node's [`DocRegistry`].
///
/// Mount it on the
/// router the node serves over iroh, behind a gate whose identity
/// resolver yields `Principal::Host { endpoint }` from the proved
/// connection.
#[derive(Clone)]
pub struct DocPeerHost<C> {
    registry: DocRegistry,
    catalog: C,
    admit_peer: Arc<PeerCheck>,
}

impl<C: DocCatalog> DocPeerHost<C> {
    /// Serve `registry`'s documents (as `catalog` lists them) to the
    /// `Principal::Host` callers whose endpoint id `admit_peer` accepts.
    /// Every other caller, and a call with no gate in front, is refused.
    pub fn new<F>(registry: DocRegistry, catalog: C, admit_peer: F) -> Self
    where
        F: Fn(&str) -> bool + Send + Sync + 'static,
    {
        Self {
            registry,
            catalog,
            admit_peer: Arc::new(admit_peer),
        }
    }

    /// The caller is an admitted node.
    fn admit(&self) -> Result<(), SyncError> {
        match architect::permissions_gate::caller() {
            Some(Principal::Host { endpoint }) if (self.admit_peer)(&endpoint) => Ok(()),
            who => {
                tracing::debug!(
                    caller = who
                        .as_ref()
                        .map_or_else(|| "none".to_owned(), Principal::describe),
                    "doc-peer: refused a caller that is not an admitted node"
                );
                Err(SyncError::Internal(
                    "the document lane is for this org's admitted nodes".into(),
                ))
            }
        }
    }
}

impl<C: DocCatalog + Clone> DocPeer for DocPeerHost<C> {
    async fn list(&self) -> Result<Vec<HeldDoc>, SyncError> {
        self.admit()?;
        self.catalog.held().await
    }

    async fn sync(
        &self,
        at: DocAt,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError> {
        self.admit()?;
        let doc_id = self.catalog.resolve(&at).await?;
        check_lineage(&self.registry, doc_id, &from).await?;
        // The registry's authorizer sees the same caller (this task).
        self.registry.sync(doc_id, from, up, down).await
    }
}

// ── Lanes ──────────────────────────────────────────────────────────────

/// Something a session can run over: the `DocSync` call, whoever answers.
///
/// A node's own registry ([`SelfLane`], or any [`DocSyncClient`]) and
/// a peer's document lane ([`PeerLane`]) are both lanes, so one session
/// shape carries every guarantee on both.
pub trait SyncLane: Sync {
    fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> impl Future<Output = Result<(), vox::VoxError<SyncError>>> + Send;
}

impl SyncLane for DocSyncClient {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), vox::VoxError<SyncError>> {
        Self::sync(self, doc_id, from, up, down).await
    }
}

/// One document on a peer's [`DocPeer`] lane, as a [`SyncLane`].
pub struct PeerLane<'a> {
    pub client: &'a DocPeerClient,
    pub name: &'a str,
}

impl SyncLane for PeerLane<'_> {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), vox::VoxError<SyncError>> {
        let at = DocAt {
            doc_id,
            name: self.name.to_owned(),
        };
        self.client.sync(at, from, up, down).await
    }
}

/// The `DocSync` method's permit for [`SelfLane`]'s gate: its engine
/// allows everything, the point of the gate is the fixed principal.
const SELF_LANE_PERMITS: ServicePermits = ServicePermits {
    service: "DocSync",
    methods: &[MethodPermit::new("sync", "write", "docs/**")],
};

/// This node's own registry as a client of itself.
///
/// It sits behind a gate that presents `who` on every call, so the
/// registry's authorizer sees a
/// principal (typically `Principal::Service { name: "doc-peer" }`)
/// rather than `None`. The in-process server task stops when this drops.
pub struct SelfLane {
    client: DocSyncClient,
    server: tokio::task::JoinHandle<()>,
}

impl SelfLane {
    /// Open the lane over `registry`, as `who`.
    pub async fn open(registry: DocRegistry, who: Principal) -> Result<Self, String> {
        let router = LayerRouter::new().with(
            crate::sync::doc_sync_service_descriptor(),
            crate::sync::DocSyncDispatcher::new(registry),
        );
        let gated = PermissionsGate::new(Arc::new(AllowAll), Arc::new(StaticPrincipal(who)))
            .permit(
                crate::sync::doc_sync_service_descriptor(),
                &SELF_LANE_PERMITS,
            )
            .wrap(router);
        let (client_link, server_link) = vox::memory_link_pair(16);
        let server = tokio::spawn(async move {
            match vox::acceptor_on(server_link)
                .on_lane(handler_acceptor(gated))
                .establish_connection()
                .await
            {
                Ok(connection) => {
                    let _hold = connection;
                    std::future::pending::<()>().await;
                }
                Err(e) => tracing::warn!(error = %e, "doc-peer: self lane acceptor failed"),
            }
        });
        match vox::initiator_on(client_link)
            .establish::<DocSyncClient>()
            .await
        {
            Ok(client) => Ok(Self { client, server }),
            Err(e) => {
                server.abort();
                Err(format!("self lane: {e:?}"))
            }
        }
    }

    /// The client half, for callers that want a plain [`DocSyncClient`].
    #[must_use]
    pub const fn client(&self) -> &DocSyncClient {
        &self.client
    }
}

impl Drop for SelfLane {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl SyncLane for SelfLane {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), vox::VoxError<SyncError>> {
        self.client.sync(doc_id, from, up, down).await
    }
}

// ── Exchange ───────────────────────────────────────────────────────────

/// How a session ended short of converging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEnd {
    /// The connection went (or never attached); try again next sweep.
    Dropped,
    /// The other side refused the attach (not admitted, not authorized,
    /// unknown doc). Refused the same way until something changes.
    Refused(String),
    /// The two histories are different documents of one id; neither is
    /// merged into the other. See [`is_foreign_lineage`].
    ForeignLineage,
}

/// What an [`exchange`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exchanged {
    /// The peer had history this node lacked; it is in the local
    /// registry now.
    Pulled,
    /// Nothing came down. What this node had and the peer lacked, if
    /// anything, went up.
    Unchanged,
}

/// Exchange one document between this node's registry (`local`) and a
/// peer's (`remote`), both directions. See the module docs.
///
/// # Errors
///
/// How a session ended short of converging ([`SessionEnd`]).
pub async fn exchange(
    doc_id: Uuid,
    local: &impl SyncLane,
    remote: &impl SyncLane,
) -> Result<Exchanged, SessionEnd> {
    let pipe = CrdtDoc::ephemeral();
    // What this node holds…
    sync_once(&pipe, doc_id, local).await?;
    let held = pipe.loro().oplog_vv();
    // …meets what the peer holds: theirs comes down into the pipe, ours
    // goes up until the peer has acknowledged all of it…
    sync_once(&pipe, doc_id, remote).await?;
    if held.includes_vv(&pipe.loro().oplog_vv()) {
        return Ok(Exchanged::Unchanged);
    }
    // …and what came down goes into the local registry.
    sync_once(&pipe, doc_id, local).await?;
    Ok(Exchanged::Pulled)
}

/// One session that ends as soon as the far side holds everything `doc`
/// has (and `doc` holds everything the far side had at attach).
async fn sync_once(doc: &CrdtDoc, doc_id: Uuid, lane: &impl SyncLane) -> Result<(), SessionEnd> {
    let (up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, mut down_rx) = vox::channel::<SyncDown>();
    let ours = doc.loro().oplog_vv();
    let mut acked = VersionVector::new();
    let mut attached = false;
    let call = lane.sync(doc_id, ours.encode(), up_rx, down_tx);
    let mut call = std::pin::pin!(call);

    loop {
        tokio::select! {
            // The call first: it sends nothing until polled, and its
            // request is what binds `up_tx`.
            biased;
            res = &mut call => return Err(classify(res)),
            incoming = down_rx.recv() => {
                let mut owned: Option<SyncDown> = None;
                match incoming {
                    Ok(Some(frame)) => {
                        let _ = frame.map(|f| owned = Some(f));
                    }
                    // The stream ended: a refused attach delivers its
                    // error as the response, which also closes the
                    // channels; read the call to say which it was.
                    _ => return Err(classify(call.await)),
                }
                match owned {
                    Some(SyncDown::Attached { server_vv, backlog }) => {
                        if !backlog.is_empty() && doc.apply_remote(&backlog).is_err() {
                            return Err(SessionEnd::Dropped);
                        }
                        let Ok(server_vv) = VersionVector::decode(&server_vv) else {
                            return Err(SessionEnd::Dropped);
                        };
                        if !server_vv.includes_vv(&ours) {
                            // History the far side lacks: push exactly it.
                            // In frames the link takes.
                            let missing = crate::sync::backlog_chunks(
                                doc.loro(),
                                &server_vv,
                                crate::sync::MAX_FRAME,
                            );
                            match missing {
                                Ok(chunks) => {
                                    for bytes in chunks {
                                        if up_tx.send(bytes).await.is_err() {
                                            return Err(SessionEnd::Dropped);
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(?doc_id, "doc-peer: export delta: {e}");
                                    return Err(SessionEnd::Dropped);
                                }
                            }
                        }
                        acked.merge(&server_vv);
                        attached = true;
                    }
                    Some(SyncDown::Update(bytes)) => {
                        if doc.apply_remote(&bytes).is_err() {
                            return Err(SessionEnd::Dropped);
                        }
                        // The far side publishes every update it applied,
                        // ours included: what comes down is held there.
                        if let Ok(meta) = LoroDoc::decode_import_blob_meta(&bytes, false) {
                            acked.merge(&meta.partial_end_vv);
                        }
                    }
                    None => continue,
                }
                if attached && acked.includes_vv(&doc.loro().oplog_vv()) {
                    return Ok(());
                }
            }
        }
    }
}

/// How a finished `sync` call ended the session.
fn classify(res: Result<(), vox::VoxError<SyncError>>) -> SessionEnd {
    res.map_or_else(classify_err, |()| SessionEnd::Dropped)
}

/// What a failed call says about the session.
fn classify_err(err: vox::VoxError<SyncError>) -> SessionEnd {
    match err {
        vox::VoxError::User(e) => match *e {
            SyncError::BadVersion(m) if m.starts_with(FOREIGN_LINEAGE) => {
                SessionEnd::ForeignLineage
            }
            other => SessionEnd::Refused(other.to_string()),
        },
        other => {
            // The gate's denial arrives as a transport-level error
            // (`InvalidPayload("permission denied: …")`): a refusal,
            // not a dropped link.
            let text = format!("{other:?}");
            if text.contains("permission denied") {
                SessionEnd::Refused(text)
            } else {
                SessionEnd::Dropped
            }
        }
    }
}

// ── Pull ───────────────────────────────────────────────────────────────

/// What one [`pull`] from one peer did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PullReport {
    /// Documents the peer holds.
    pub listed: usize,
    /// Documents whose digest differed here, so were exchanged.
    pub exchanged: usize,
    /// Of those, documents the peer had history of that this node lacked.
    pub pulled: usize,
    /// Two nodes that made two different documents of one id; left as
    /// they are on both.
    pub foreign: usize,
    /// Refused, dropped or timed out; tried again next sweep.
    pub failed: usize,
}

/// Pull `peer`'s documents into this node and push ours to it.
///
/// Runs through `local` (this node's own registry); both directions
/// travel in the same sessions. `ours` maps
/// this node's doc ids to their digests; an entry whose non-empty digest
/// matches is skipped. Each exchange is bounded by `per_doc`, so one
/// stuck session cannot stall the sweep.
///
/// # Errors
///
/// The peer's `list` failed: unreachable, or this node is not admitted.
pub async fn pull<S: BuildHasher + Sync>(
    local: &impl SyncLane,
    peer: &DocPeerClient,
    ours: &HashMap<Uuid, String, S>,
    per_doc: Duration,
) -> Result<PullReport, SessionEnd> {
    let theirs = peer.list().await.map_err(classify_err)?;
    let mut report = PullReport {
        listed: theirs.len(),
        ..PullReport::default()
    };
    for entry in &theirs {
        if !entry.digest.is_empty() && ours.get(&entry.doc_id) == Some(&entry.digest) {
            continue;
        }
        report.exchanged = report.exchanged.saturating_add(1);
        let remote = PeerLane {
            client: peer,
            name: &entry.name,
        };
        match tokio::time::timeout(per_doc, exchange(entry.doc_id, local, &remote)).await {
            Ok(Ok(Exchanged::Pulled)) => report.pulled = report.pulled.saturating_add(1),
            Ok(Ok(Exchanged::Unchanged)) => {}
            Ok(Err(SessionEnd::ForeignLineage)) => {
                report.foreign = report.foreign.saturating_add(1);
            }
            Ok(Err(_)) | Err(_) => report.failed = report.failed.saturating_add(1),
        }
    }
    if report.foreign > 0 {
        // Alertable: two documents of one id will not converge until
        // somebody decides which to keep.
        tracing::warn!(
            foreign = report.foreign,
            "doc-peer: documents made separately on two nodes are not merged"
        );
    }
    Ok(report)
}
