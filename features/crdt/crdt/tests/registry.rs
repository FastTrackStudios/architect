//! `DocRegistry` tests: one mounted DocSync/DocPresence dispatcher pair
//! serving many docs over the in-process vox transport — concurrent
//! convergence per doc (with isolation between docs), the admission
//! gate, and idle eviction followed by a persistence-backed reopen.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
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
