//! `DocRegistry` tests: one mounted DocSync/DocPresence dispatcher pair
//! serving many docs over the in-process vox transport — concurrent
//! convergence per doc (with isolation between docs), the admission
//! gate, idle eviction followed by a persistence-backed reopen, and a
//! reader (admitted to read, not to write) following a doc and its
//! presence without changing either.

// This is an integration-test crate. `clippy.toml`'s
// `allow-*-in-tests` only reaches `#[test]` fns and `#[cfg(test)]`
// modules, so the fixture/mock `impl` blocks below — where an
// `unwrap()` IS the assertion — still trip the panic lints. Allow
// them crate-wide here rather than dotting the file with attributes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::panic
)]
#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::time::Duration;

use architect::layer::handler_acceptor;
use architect::permissions_gate::{PermissionsGate, StaticPrincipal, UnlistedPolicy};
use architect::{LayerRouter, LocalServer, Scope};
use architect_permissions::{AllowAll, Principal};
use crdt::CrdtDoc;
use crdt::registry::DocRegistry;
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, DocSyncClient, DocSyncDispatcher, PresencePeer,
    SyncError, SyncedDoc, doc_presence_service_descriptor, doc_sync_service_descriptor,
};
use uuid::Uuid;

/// Poll until `pred` holds on the doc (or panic after 5s).
async fn converge(doc: &CrdtDoc, what: &str, pred: impl Fn(&loro::LoroDoc) -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if pred(doc.loro()) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Poll until `pred` (or panic after 5s).
async fn eventually(what: &str, mut pred: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !pred().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn map_value(doc: &loro::LoroDoc, key: &str) -> Option<String> {
    doc.get_map("root")
        .get(key)
        .and_then(|v| v.into_value().ok())
        .and_then(|v| v.into_string().ok())
        .map(|s| s.to_string())
}

/// Registry over throwaway in-memory docs.
fn ephemeral_registry() -> DocRegistry {
    DocRegistry::new(|_doc_id| Box::pin(async { Ok(CrdtDoc::ephemeral()) }))
}

/// Mount both services for `registry` on an in-process server.
fn serve(registry: &DocRegistry, scope: &std::sync::Arc<Scope>) -> LocalServer {
    let router = LayerRouter::new()
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(registry.clone()),
        )
        .with(
            doc_presence_service_descriptor(),
            DocPresenceDispatcher::new(registry.clone()),
        );
    LocalServer::serve(router, scope.clone())
}

/// Attach a fresh ephemeral replica of `doc_id`; returns the doc and
/// the session task (abort it to "disconnect" the replica).
async fn attach_replica(
    local: &LocalServer,
    doc_id: Uuid,
) -> (CrdtDoc, tokio::task::JoinHandle<()>) {
    let client: DocSyncClient = local.establish().await.expect("establish sync client");
    let mut synced = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
    let doc = synced.doc().clone();
    let session = tokio::spawn(async move {
        let _ = synced.run(&client).await;
    });
    (doc, session)
}

/// One dispatcher pair serves two docs at once: each doc's replicas
/// converge among themselves, and nothing bleeds between docs.
#[tokio::test(flavor = "multi_thread")]
async fn registry_serves_two_docs_concurrently() {
    let scope = Scope::new();
    let registry = ephemeral_registry();
    let local = serve(&registry, &scope);

    let doc_x = Uuid::new_v4();
    let doc_y = Uuid::new_v4();

    let (x1, _) = attach_replica(&local, doc_x).await;
    let (x2, _) = attach_replica(&local, doc_x).await;
    let (y1, _) = attach_replica(&local, doc_y).await;
    let (y2, _) = attach_replica(&local, doc_y).await;

    // Concurrent writes on both docs.
    x1.loro()
        .get_map("root")
        .insert("from-x", "hello")
        .expect("insert x");
    x1.loro().commit();
    y1.loro()
        .get_map("root")
        .insert("from-y", "world")
        .expect("insert y");
    y1.loro().commit();

    converge(&x2, "x replicas converge", |d| {
        map_value(d, "from-x").as_deref() == Some("hello")
    })
    .await;
    converge(&y2, "y replicas converge", |d| {
        map_value(d, "from-y").as_deref() == Some("world")
    })
    .await;

    // Isolation: the docs are distinct boundaries served by one router.
    assert_eq!(map_value(x2.loro(), "from-y"), None, "doc bleed x<-y");
    assert_eq!(map_value(y2.loro(), "from-x"), None, "doc bleed y<-x");

    let mut open = registry.open_docs().await;
    open.sort();
    let mut expected = vec![doc_x, doc_y];
    expected.sort();
    assert_eq!(open, expected);

    scope.close().await;
}

/// Presence routes per doc through the same registry dispatcher.
#[tokio::test(flavor = "multi_thread")]
async fn registry_presence_is_per_doc() {
    let scope = Scope::new();
    let registry = ephemeral_registry();
    let local = serve(&registry, &scope);

    let doc_x = Uuid::new_v4();
    let doc_y = Uuid::new_v4();

    let mut peers = Vec::new();
    for (doc_id, key, value) in [
        (doc_x, "ax", "alice"),
        (doc_x, "bx", "bob"),
        (doc_y, "cy", "carol"),
    ] {
        let client: DocPresenceClient = local.establish().await.expect("presence client");
        let (peer, mut driver) = PresencePeer::new(doc_id, 30_000);
        tokio::spawn(async move {
            let _ = driver.run(&client).await;
        });
        peer.set(key, value);
        peers.push(peer);
    }
    let (alice, bob, carol) = (&peers[0], &peers[1], &peers[2]);

    eventually("bob sees alice", async || bob.states().contains_key("ax")).await;
    eventually("alice sees bob", async || alice.states().contains_key("bx")).await;
    // Carol is on the other doc: she sees neither, they don't see her.
    assert!(!carol.states().contains_key("ax"));
    assert!(!bob.states().contains_key("cy"));

    scope.close().await;
}

/// A peer that sits still stays: its driver writes its keys again well
/// inside the timeout. One that leaves goes: the others' drivers prune it
/// once it has outlived the timeout.
#[tokio::test(flavor = "multi_thread")]
async fn presence_keeps_a_still_peer_and_drops_a_gone_one() {
    const TIMEOUT_MS: i64 = 1_500;
    let scope = Scope::new();
    let registry = ephemeral_registry().with_presence_timeout(TIMEOUT_MS);
    let local = serve(&registry, &scope);
    let doc_id = Uuid::new_v4();

    let mut drivers = Vec::new();
    let mut peers = Vec::new();
    for (key, value) in [("a", "alice"), ("b", "bob")] {
        let client: DocPresenceClient = local.establish().await.expect("presence client");
        let (peer, mut driver) = PresencePeer::new(doc_id, TIMEOUT_MS);
        drivers.push(tokio::spawn(async move {
            let _ = driver.run(&client).await;
        }));
        peer.set(key, value);
        peers.push(peer);
    }
    let bob = &peers[1];
    eventually("bob sees alice", async || bob.states().contains_key("a")).await;

    // Twice the timeout with nothing new from alice: still here.
    tokio::time::sleep(Duration::from_millis(2 * TIMEOUT_MS as u64)).await;
    assert!(bob.states().contains_key("a"), "a still peer is kept alive");

    // Alice goes.
    drivers[0].abort();
    eventually("bob no longer sees alice", async || {
        !bob.states().contains_key("a")
    })
    .await;
    assert!(bob.states().contains_key("b"), "bob keeps himself");

    scope.close().await;
}

/// The admission hook gates opening *and* serving: a rejected id fails
/// with `UnknownDoc` and never reaches the factory.
#[tokio::test(flavor = "multi_thread")]
async fn admission_hook_rejects_unknown_docs() {
    let scope = Scope::new();
    let allowed = Uuid::new_v4();
    let denied = Uuid::new_v4();
    let registry = ephemeral_registry().with_admission(move |doc_id| doc_id == allowed);
    let local = serve(&registry, &scope);

    let client: DocSyncClient = local.establish().await.expect("client");

    // Denied id: rejected before the factory ever runs — an attach
    // failure resolves the call immediately (only a successful attach
    // holds it open).
    let (_up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, _down_rx) = vox::channel::<crdt::sync::SyncDown>();
    let err = client
        .sync(denied, Vec::new(), up_rx, down_tx)
        .await
        .expect_err("denied doc must not attach");
    assert!(
        matches!(&err, vox::VoxError::User(boxed) if matches!(**boxed, SyncError::UnknownDoc)),
        "expected UnknownDoc, got: {err:?}"
    );
    assert!(!registry.is_open(denied).await, "denied doc was opened");

    // Allowed id: attaches normally — the Attached frame arrives while
    // the call stays in flight; closing the up channel ends the session.
    let (up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, mut down_rx) = vox::channel::<crdt::sync::SyncDown>();
    let mut call = std::pin::pin!(client.sync(allowed, Vec::new(), up_rx, down_tx));
    tokio::select! {
        res = &mut call => panic!("sync call ended before the session: {res:?}"),
        frame = down_rx.recv() => {
            frame.expect("down channel error").expect("down closed before attach");
        }
    }
    assert!(registry.is_open(allowed).await);
    drop(up_tx);
    call.await.expect("allowed session ends cleanly");

    scope.close().await;
}

/// Idle eviction drops a doc's hosts once its last session closes, and
/// the next attach reopens it from persistence with nothing lost.
#[tokio::test(flavor = "multi_thread")]
async fn eviction_then_reopen_preserves_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let doc_id = Uuid::new_v4();
    let scope = Scope::new();

    // FilePersistence namespaces by doc id under `root` on its own.
    let registry = DocRegistry::new(move |id| {
        let root = root.clone();
        Box::pin(async move { CrdtDoc::open(id, crdt::FilePersistence::new(root)).await })
    })
    .with_idle_eviction(Duration::from_millis(200));
    let local = serve(&registry, &scope);

    // A replica writes; a second replica converging proves the server
    // applied (and durably persisted) the update.
    let (doc_a, session_a) = attach_replica(&local, doc_id).await;
    let (doc_b, session_b) = attach_replica(&local, doc_id).await;
    doc_a
        .loro()
        .get_map("root")
        .insert("k", "v")
        .expect("insert");
    doc_a.loro().commit();
    converge(&doc_b, "server relayed the write", |d| {
        map_value(d, "k").as_deref() == Some("v")
    })
    .await;

    // Disconnect both replicas → zero sessions → the sweeper compacts
    // and evicts.
    session_a.abort();
    session_b.abort();
    eventually("idle doc evicted", async || !registry.is_open(doc_id).await).await;

    // A fresh replica attaches: the factory reopens from persistence
    // and full history comes down.
    let (doc_c, _) = attach_replica(&local, doc_id).await;
    converge(&doc_c, "reopened doc preserves data", |d| {
        map_value(d, "k").as_deref() == Some("v")
    })
    .await;
    assert!(registry.is_open(doc_id).await);

    scope.close().await;
}

// ── Readers ─────────────────────────────────────────────────────────────

fn user(id: &str) -> Principal {
    Principal::User {
        user_id: id.to_owned(),
    }
}

/// A registry everyone may open and only `writer` may write.
fn readers_registry(writer: &str) -> DocRegistry {
    let writer = user(writer);
    ephemeral_registry().with_write_authorizer(move |who, _| who == Some(&writer))
}

/// A client of `registry`'s two services on its own in-process
/// connection, calling as `who` (a real gate resolving it).
async fn client_as<C: vox::FromVoxLane>(registry: &DocRegistry, who: &Principal) -> C {
    let router = LayerRouter::new()
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(registry.clone()),
        )
        .with(
            doc_presence_service_descriptor(),
            DocPresenceDispatcher::new(registry.clone()),
        );
    let gated = PermissionsGate::new(Arc::new(AllowAll), Arc::new(StaticPrincipal(who.clone())))
        .unlisted(UnlistedPolicy::Allow)
        .wrap(router);
    let (client_link, server_link) = vox::memory_link_pair(64);
    tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(gated))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    vox::initiator_on(client_link)
        .establish::<C>()
        .await
        .expect("establish")
}

/// An update setting `key` to `value`, made on a doc of its own.
fn update(key: &str, value: &str) -> Vec<u8> {
    let doc = loro::LoroDoc::new();
    doc.get_map("root").insert(key, value).unwrap();
    doc.commit();
    doc.export(loro::ExportMode::all_updates()).unwrap()
}

/// A reader follows a writer's edits live; an update it sends up anyway
/// ends its session with `Forbidden` and leaves the doc as it was. A
/// reader running `SyncedDoc::read_only` keeps its session: its own
/// edits stay on its replica.
#[tokio::test(flavor = "multi_thread")]
async fn a_reader_follows_live_and_cannot_write() {
    let registry = readers_registry("ann");
    let doc_id = Uuid::new_v4();

    let ann: DocSyncClient = client_as(&registry, &user("ann")).await;
    let mut at_writer = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
    let writes = at_writer.doc().clone();
    tokio::spawn(async move {
        let _ = at_writer.run(&ann).await;
    });

    // A well-behaved reader.
    let reader: DocSyncClient = client_as(&registry, &user("bob")).await;
    let mut at_reader = SyncedDoc::new(doc_id, CrdtDoc::ephemeral()).read_only();
    let reads = at_reader.doc().clone();
    let following = tokio::spawn(async move { at_reader.run(&reader).await });

    writes
        .loro()
        .get_map("root")
        .insert("title", "draft")
        .unwrap();
    writes.loro().commit();
    converge(&reads, "the reader gets the writer's edit", |d| {
        map_value(d, "title").as_deref() == Some("draft")
    })
    .await;
    writes
        .loro()
        .get_map("root")
        .insert("title", "final")
        .unwrap();
    writes.loro().commit();
    converge(&reads, "the reader follows live", |d| {
        map_value(d, "title").as_deref() == Some("final")
    })
    .await;

    // Its own edit stays on its replica, and its session goes on.
    reads.loro().get_map("root").insert("note", "mine").unwrap();
    reads.loro().commit();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!following.is_finished(), "a read-only replica was cut off");
    let at_host = registry.doc(doc_id).await.unwrap();
    assert_eq!(map_value(at_host.loro(), "note"), None);

    // A reader that sends up anyway: refused, nothing applied.
    let rogue: DocSyncClient = client_as(&registry, &user("eve")).await;
    let (up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, mut down_rx) = vox::channel::<crdt::sync::SyncDown>();
    let mut call = std::pin::pin!(rogue.sync(doc_id, Vec::new(), up_rx, down_tx));
    tokio::select! {
        res = &mut call => panic!("a reader's sync ended before attaching: {res:?}"),
        frame = down_rx.recv() => {
            frame.expect("down channel").expect("attached");
        }
    }
    let send = up_tx.send(update("title", "defaced"));
    let (pushed, ended) = tokio::join!(send, tokio::time::timeout(Duration::from_secs(5), call));
    pushed.expect("the update went up");
    let err = ended
        .expect("the reader's session ended")
        .expect_err("a reader's write must be refused");
    assert!(
        matches!(&err, vox::VoxError::User(boxed) if matches!(**boxed, SyncError::Forbidden)),
        "expected Forbidden, got: {err:?}"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(map_value(at_host.loro(), "title").as_deref(), Some("final"));
    assert_eq!(map_value(writes.loro(), "title").as_deref(), Some("final"));
}

/// A reader's presence shows it everyone, the leader's playhead
/// included, and the shared clock answers it; what it publishes reaches
/// nobody.
#[tokio::test(flavor = "multi_thread")]
async fn a_readers_presence_reads_and_publishes_nothing() {
    let registry = readers_registry("ann");
    let doc_id = Uuid::new_v4();

    let mut peers = Vec::new();
    for who in ["ann", "bob", "cy"] {
        let client: DocPresenceClient = client_as(&registry, &user(who)).await;
        let (peer, mut driver) = PresencePeer::new(doc_id, 30_000);
        tokio::spawn(async move {
            let _ = driver.run(&client).await;
        });
        peers.push(peer);
    }
    // ann leads; bob and cy only read. Neither ann nor cy may see bob.
    let (lead, reader) = (&peers[0], &peers[1]);

    lead.set("lead/play", "playing at 10");
    eventually("the reader sees the leader's transport", async || {
        reader.states().contains_key("lead/play")
    })
    .await;
    let clock = reader.clock().clone();
    eventually("the shared clock answers a reader", async || {
        clock.offset_ms().is_some()
    })
    .await;

    // The reader tries to take the transport, then the leader publishes
    // again: once that reaches the reader, the reader's publish has had
    // its chance to reach the leader.
    reader.set("lead/play", "stopped by bob");
    reader.set("bob/state", "here");
    tokio::time::sleep(Duration::from_millis(200)).await;
    lead.set("ann/state", "here");
    eventually("the reader sees the leader's next state", async || {
        reader.states().contains_key("ann/state")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !lead.states().contains_key("bob/state"),
        "a reader's presence reached the leader"
    );
    assert_eq!(
        lead.states().get("lead/play"),
        Some(&loro::LoroValue::from("playing at 10")),
        "a reader took the transport"
    );
    assert!(!peers[2].states().contains_key("bob/state"));
}

/// A lane that serves `registry` to callers it knows may only read (a
/// vault share link's guests): its sessions are read-only whatever the
/// registry's write rules say about the caller.
#[derive(Clone)]
struct ReadOnlyLane(DocRegistry);

impl crdt::sync::DocSync for ReadOnlyLane {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<crdt::sync::SyncDown>,
    ) -> Result<(), SyncError> {
        self.0.sync_read_only(doc_id, from, up, down).await
    }
}

impl crdt::sync::DocPresence for ReadOnlyLane {
    async fn presence(
        &self,
        doc_id: Uuid,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), SyncError> {
        self.0.presence_read_only(doc_id, up, down).await
    }

    async fn now(&self) -> f64 {
        crdt::presence::clock_ms()
    }
}

/// A client of `lane` on its own in-process connection, calling as
/// `who` (a real gate resolving it).
async fn lane_client_as<C: vox::FromVoxLane>(lane: &ReadOnlyLane, who: &Principal) -> C {
    let router = LayerRouter::new()
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(lane.clone()),
        )
        .with(
            doc_presence_service_descriptor(),
            DocPresenceDispatcher::new(lane.clone()),
        );
    let gated = PermissionsGate::new(Arc::new(AllowAll), Arc::new(StaticPrincipal(who.clone())))
        .unlisted(UnlistedPolicy::Allow)
        .wrap(router);
    let (client_link, server_link) = vox::memory_link_pair(64);
    tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(gated))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    vox::initiator_on(client_link)
        .establish::<C>()
        .await
        .expect("establish")
}

/// Through a read-only lane even a caller the registry lets write (ann)
/// only follows: an update she sends up ends her session with
/// `Forbidden`, and what she publishes in presence reaches nobody, while
/// she sees the others and the shared clock answers her.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_lane_keeps_even_a_writer_from_writing() {
    let registry = readers_registry("ann");
    let lane = ReadOnlyLane(registry.clone());
    let doc_id = Uuid::new_v4();
    let at_host = registry.doc(doc_id).await.unwrap();
    at_host
        .loro()
        .get_map("root")
        .insert("title", "kept")
        .unwrap();
    at_host.loro().commit();

    let ann: DocSyncClient = lane_client_as(&lane, &user("ann")).await;
    let (up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, mut down_rx) = vox::channel::<crdt::sync::SyncDown>();
    let mut call = std::pin::pin!(ann.sync(doc_id, Vec::new(), up_rx, down_tx));
    tokio::select! {
        res = &mut call => panic!("the session ended before attaching: {res:?}"),
        frame = down_rx.recv() => {
            frame.expect("down channel").expect("attached");
        }
    }
    let send = up_tx.send(update("title", "defaced"));
    let (pushed, ended) = tokio::join!(send, tokio::time::timeout(Duration::from_secs(5), call));
    pushed.expect("the update went up");
    let err = ended
        .expect("the session ended")
        .expect_err("a write through a read-only lane must be refused");
    assert!(
        matches!(&err, vox::VoxError::User(boxed) if matches!(**boxed, SyncError::Forbidden)),
        "expected Forbidden, got: {err:?}"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(map_value(at_host.loro(), "title").as_deref(), Some("kept"));

    // Presence: bob leads on the registry's own lane; ann, through the
    // read-only lane, sees him and publishes nothing.
    let bob_client: DocPresenceClient = client_as(&registry, &user("ann")).await;
    let (bob, mut bob_driver) = PresencePeer::new(doc_id, 30_000);
    tokio::spawn(async move {
        let _ = bob_driver.run(&bob_client).await;
    });
    let ann_client: DocPresenceClient = lane_client_as(&lane, &user("ann")).await;
    let (ann, mut ann_driver) = PresencePeer::new(doc_id, 30_000);
    tokio::spawn(async move {
        let _ = ann_driver.run(&ann_client).await;
    });
    bob.set("lead/play", "playing at 10");
    eventually("ann sees the leader's transport", async || {
        ann.states().contains_key("lead/play")
    })
    .await;
    let clock = ann.clock().clone();
    eventually("the shared clock answers through the lane", async || {
        clock.offset_ms().is_some()
    })
    .await;
    ann.set("lead/play", "stopped by ann");
    ann.set("ann/state", "here");
    tokio::time::sleep(Duration::from_millis(200)).await;
    bob.set("bob/state", "here");
    eventually("ann sees the leader's next state", async || {
        ann.states().contains_key("bob/state")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !bob.states().contains_key("ann/state"),
        "a read-only lane's presence reached the leader"
    );
    assert_eq!(
        bob.states().get("lead/play"),
        Some(&loro::LoroValue::from("playing at 10")),
        "a read-only lane took the transport"
    );
}
