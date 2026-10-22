//! `crdt::peer`: documents move between nodes pairwise, with no hub.
//!
//! Every node here is a `DocRegistry` behind its own `DocPeerHost`; there
//! is no `DocSyncHost` anywhere. Peer lanes are served behind a real
//! `PermissionsGate` whose resolver presents `Principal::Host` (what an
//! iroh lane resolves from the proved endpoint id), so the admission and
//! the registry's authorizer see exactly what production hands them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::panic
)]
#![cfg(not(target_arch = "wasm32"))]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::{PermissionsGate, StaticPrincipal};
use architect_permissions::{AllowAll, MethodPermit, Principal, ServicePermits};
use crdt::CrdtDoc;
use crdt::peer::{
    DocCatalog, DocPeerClient, DocPeerDispatcher, DocPeerHost, Exchanged, OpenDocs, PeerLane,
    SelfLane, SessionEnd, doc_peer_service_descriptor, exchange, pull,
};
use crdt::registry::DocRegistry;
use proptest::prelude::*;
use uuid::Uuid;

const PEER_PERMITS: ServicePermits = ServicePermits {
    service: "DocPeer",
    methods: &[
        MethodPermit::new("list", "read", "docs/**"),
        MethodPermit::new("sync", "write", "docs/**"),
    ],
};

const PER_DOC: Duration = Duration::from_secs(10);

/// One node: a registry of in-memory docs, the peers it admits, and the
/// principals its authorizer was asked about.
struct Node {
    name: String,
    registry: DocRegistry,
    seen: Arc<Mutex<Vec<String>>>,
}

/// A node that admits `peers` (by endpoint id) to every doc except those
/// in `private`, and its own self lane (`service:<name>`) to all.
fn node(name: &str, peers: &[&str], private: &[Uuid]) -> Node {
    let me = name.to_owned();
    let peers: HashSet<String> = peers.iter().map(|p| (*p).to_owned()).collect();
    let private: HashSet<Uuid> = private.iter().copied().collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let registry = DocRegistry::new(|_| Box::pin(async { Ok(CrdtDoc::ephemeral()) }))
        .with_authorizer(move |who, doc_id| {
            log.lock()
                .unwrap()
                .push(who.map_or_else(|| "none".to_owned(), Principal::describe));
            match who {
                Some(Principal::Service { name }) => *name == me,
                Some(Principal::Host { endpoint }) => {
                    peers.contains(endpoint) && !private.contains(&doc_id)
                }
                _ => false,
            }
        });
    Node {
        name: name.to_owned(),
        registry,
        seen,
    }
}

/// `node`'s peer lane as seen by a caller the gate resolves to `who`.
async fn dial(node: &Node, who: Principal) -> (DocPeerClient, tokio::task::JoinHandle<()>) {
    let admitted: HashSet<String> = ["a", "b", "c"].iter().map(|s| (*s).to_owned()).collect();
    let host = DocPeerHost::new(
        node.registry.clone(),
        OpenDocs(node.registry.clone()),
        move |endpoint| admitted.contains(endpoint),
    );
    let router = LayerRouter::new().with(
        doc_peer_service_descriptor(),
        DocPeerDispatcher::new(host),
    );
    let gated = PermissionsGate::new(Arc::new(AllowAll), Arc::new(StaticPrincipal(who)))
        .permit(doc_peer_service_descriptor(), &PEER_PERMITS)
        .wrap(router);
    let (client_link, server_link) = vox::memory_link_pair(16);
    let server = tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(gated))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    let client = vox::initiator_on(client_link)
        .establish::<DocPeerClient>()
        .await
        .expect("establish peer lane");
    (client, server)
}

fn host(endpoint: &str) -> Principal {
    Principal::Host {
        endpoint: endpoint.to_owned(),
    }
}

async fn self_lane(node: &Node) -> SelfLane {
    SelfLane::open(
        node.registry.clone(),
        Principal::Service {
            name: node.name.clone(),
        },
    )
    .await
    .expect("self lane")
}

async fn edit(node: &Node, doc_id: Uuid, key: &str, value: &str) {
    let doc = node.registry.doc(doc_id).await.expect("open doc");
    doc.loro().get_map("root").insert(key, value).unwrap();
    doc.loro().commit();
}

async fn value(node: &Node, doc_id: Uuid, key: &str) -> Option<String> {
    let doc = node.registry.doc(doc_id).await.expect("open doc");
    doc.loro()
        .get_map("root")
        .get(key)
        .and_then(|v| v.into_value().ok())
        .and_then(|v| v.into_string().ok())
        .map(|s| s.to_string())
}

async fn digests(node: &Node) -> HashMap<Uuid, String> {
    OpenDocs(node.registry.clone())
        .held()
        .await
        .unwrap()
        .into_iter()
        .map(|h| (h.doc_id, h.digest))
        .collect()
}

/// `puller` pulls everything `from` holds (and pushes back in the same
/// sessions).
async fn sweep(puller: &Node, from: &Node) -> crdt::peer::PullReport {
    let local = self_lane(puller).await;
    let (peer, server) = dial(from, host(&puller.name)).await;
    let ours = digests(puller).await;
    let report = pull(&local, &peer, &ours, PER_DOC).await.expect("pull");
    server.abort();
    report
}

// t[verify crdt.peer] — A→B, B→C with A offline and no hub: C converges.
#[tokio::test(flavor = "multi_thread")]
async fn three_replicas_converge_without_a_hub() {
    let doc_id = Uuid::new_v4();
    let a = node("a", &["b", "c"], &[]);
    let b = node("b", &["a", "c"], &[]);
    let c = node("c", &["a", "b"], &[]);

    // One document, started on A and carried to B and C (a doc made
    // separately on each node would be a foreign lineage, refused).
    edit(&a, doc_id, "seed", "0").await;
    assert_eq!(sweep(&b, &a).await.pulled, 1);
    assert_eq!(sweep(&c, &b).await.pulled, 1);

    // Concurrent edits on all three.
    edit(&a, doc_id, "from_a", "1").await;
    edit(&b, doc_id, "from_b", "2").await;
    edit(&c, doc_id, "from_c", "3").await;

    // B pulls from A: both directions in one sweep.
    let r = sweep(&b, &a).await;
    assert_eq!((r.listed, r.exchanged, r.pulled, r.failed), (1, 1, 1, 0));
    assert_eq!(value(&b, doc_id, "from_a").await.as_deref(), Some("1"));
    assert_eq!(value(&a, doc_id, "from_b").await.as_deref(), Some("2"));

    // A goes offline (never dialled again); C pulls from B.
    let r = sweep(&c, &b).await;
    assert_eq!((r.pulled, r.failed, r.foreign), (1, 0, 0));
    for key in ["from_a", "from_b", "from_c"] {
        assert!(value(&c, doc_id, key).await.is_some(), "c lacks {key}");
        assert!(value(&b, doc_id, key).await.is_some(), "b lacks {key}");
    }
    assert_eq!(value(&a, doc_id, "from_c").await, None, "a was offline");

    // A comes back and pulls from C, which it never exchanged with before.
    let r = sweep(&a, &c).await;
    assert_eq!(r.pulled, 1);
    let vv = |n: &Node| {
        let reg = n.registry.clone();
        async move { reg.doc(doc_id).await.unwrap().loro().oplog_vv() }
    };
    assert_eq!(vv(&a).await, vv(&b).await);
    assert_eq!(vv(&b).await, vv(&c).await);

    // Nothing left to move: equal digests are skipped.
    let r = sweep(&b, &a).await;
    assert_eq!((r.listed, r.exchanged), (1, 0));
}

// t[verify crdt.peer.admission] — a host the node does not admit gets
// nothing: no list, no session, and the registry's doc is untouched.
#[tokio::test(flavor = "multi_thread")]
async fn an_unadmitted_host_is_refused() {
    let doc_id = Uuid::new_v4();
    let a = node("a", &["b"], &[]);
    edit(&a, doc_id, "secret", "x").await;
    let intruder = node("intruder", &[], &[]);

    let (peer, server) = dial(&a, host("intruder")).await;
    assert!(peer.list().await.is_err(), "list must be refused");
    let local = self_lane(&intruder).await;
    let remote = PeerLane {
        client: &peer,
        name: "anything",
    };
    let end = exchange(doc_id, &local, &remote).await;
    assert!(matches!(end, Err(SessionEnd::Refused(_))), "got {end:?}");
    assert_eq!(value(&intruder, doc_id, "secret").await, None);
    server.abort();

    // A signed-in user is not a node either.
    let (peer, server) = dial(
        &a,
        Principal::User {
            user_id: "u1".into(),
        },
    )
    .await;
    assert!(peer.list().await.is_err(), "a user is not a node");
    server.abort();
}

// t[verify crdt.registry.authorizer] — the registry's authorizer receives
// the gate's caller and decides per document.
#[tokio::test(flavor = "multi_thread")]
async fn the_registry_authorizer_sees_the_gates_caller() {
    let shared = Uuid::new_v4();
    let private = Uuid::new_v4();
    let a = node("a", &["b"], &[private]);
    let b = node("b", &["a"], &[]);
    edit(&a, shared, "k", "shared").await;
    edit(&a, private, "k", "private").await;

    let local = self_lane(&b).await;
    let (peer, server) = dial(&a, host("b")).await;
    let ok = exchange(
        shared,
        &local,
        &PeerLane {
            client: &peer,
            name: "shared",
        },
    )
    .await;
    assert_eq!(ok, Ok(Exchanged::Pulled));
    let refused = exchange(
        private,
        &local,
        &PeerLane {
            client: &peer,
            name: "private",
        },
    )
    .await;
    assert!(matches!(refused, Err(SessionEnd::Refused(_))), "got {refused:?}");
    assert_eq!(value(&b, private, "k").await, None);
    server.abort();

    let seen = a.seen.lock().unwrap().clone();
    assert!(seen.iter().any(|p| p == "host:b"), "a's authorizer saw {seen:?}");
    assert!(!seen.iter().any(|p| p == "none"), "a lane without a caller: {seen:?}");
    let seen_b = b.seen.lock().unwrap().clone();
    assert!(seen_b.iter().all(|p| p == "service:b"), "b's self lane: {seen_b:?}");
}

// t[verify crdt.peer.lineage] — two documents made separately under one
// id are refused, not merged.
#[tokio::test(flavor = "multi_thread")]
async fn a_foreign_lineage_is_refused() {
    let doc_id = Uuid::new_v4();
    let a = node("a", &["b"], &[]);
    let b = node("b", &["a"], &[]);
    edit(&a, doc_id, "k", "a's").await;
    edit(&b, doc_id, "k", "b's").await;
    // Distinct peers with no shared history: B's sweep counts it foreign.
    let r = sweep(&b, &a).await;
    assert_eq!((r.exchanged, r.foreign, r.pulled), (1, 1, 0));
    assert_eq!(value(&b, doc_id, "k").await.as_deref(), Some("b's"));
    assert_eq!(value(&a, doc_id, "k").await.as_deref(), Some("a's"));
}

#[derive(Debug, Clone)]
enum Op {
    Edit { node: usize, key: u8, value: u8 },
    Exchange { puller: usize, from: usize },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..3usize, 0..4u8, any::<u8>()).prop_map(|(node, key, value)| Op::Edit {
            node,
            key,
            value
        }),
        (0..3usize, 0..3usize)
            .prop_filter("a node does not pull from itself", |(p, f)| p != f)
            .prop_map(|(puller, from)| Op::Exchange { puller, from }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    // t[verify crdt.peer.convergence] — any order of edits and pairwise
    // exchanges, followed by a sweep around the ring, leaves all three
    // nodes holding the same document.
    #[test]
    fn any_edit_order_converges(ops in prop::collection::vec(op(), 1..24)) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let doc_id = Uuid::new_v4();
            let nodes = [
                node("a", &["b", "c"], &[]),
                node("b", &["a", "c"], &[]),
                node("c", &["a", "b"], &[]),
            ];
            // Every node starts from one shared seed, as two devices of
            // one org do (separately made documents are refused instead).
            edit(&nodes[0], doc_id, "seed", "0").await;
            sweep(&nodes[1], &nodes[0]).await;
            sweep(&nodes[2], &nodes[0]).await;

            for op in &ops {
                match *op {
                    Op::Edit { node, key, value } => {
                        edit(&nodes[node], doc_id, &format!("k{key}"), &value.to_string()).await;
                    }
                    Op::Exchange { puller, from } => {
                        let r = sweep(&nodes[puller], &nodes[from]).await;
                        assert_eq!((r.failed, r.foreign), (0, 0), "{op:?}");
                    }
                }
            }
            // Settle: around the ring and back.
            for (p, f) in [(1, 0), (2, 1), (0, 2), (1, 0)] {
                sweep(&nodes[p], &nodes[f]).await;
            }
            let mut states = Vec::new();
            for n in &nodes {
                let doc = n.registry.doc(doc_id).await.unwrap();
                states.push((doc.loro().oplog_vv(), doc.loro().get_deep_value()));
            }
            assert_eq!(states[0], states[1], "a vs b after {ops:?}");
            assert_eq!(states[1], states[2], "b vs c after {ops:?}");
        });
    }
}
