//! Convergence test for the doc-sync transport: two live replicas and a
//! late joiner, all syncing one canonical doc through `DocSyncHost` over
//! the in-process vox transport. Local edits on any replica must appear
//! on every other; a replica that connects late (or reconnects) must
//! catch up by version vector.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
use crdt::CrdtDoc;
use crdt::sync::{DocSyncClient, DocSyncHost, SyncedDoc, doc_sync_service_descriptor};
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

fn map_value(doc: &loro::LoroDoc, key: &str) -> Option<String> {
    doc.get_map("root")
        .get(key)
        .and_then(|v| v.into_value().ok())
        .and_then(|v| v.into_string().ok())
        .map(|s| s.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn replicas_converge_and_late_joiners_catch_up() {
    let doc_id = Uuid::new_v4();
    let scope = Scope::new();

    // Canonical doc + sync host, served in-process.
    let canonical = CrdtDoc::ephemeral();
    let host = DocSyncHost::new(doc_id, canonical.clone());
    let router = LayerRouter::new().with(
        doc_sync_service_descriptor(),
        crdt::sync::DocSyncDispatcher::new(host),
    );
    let local = LocalServer::serve(router, scope.clone());

    // Replica A and B: fresh docs, each on its own session.
    let client_a: DocSyncClient = local.establish().await.expect("client a");
    let mut a = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
    let doc_a = a.doc().clone();
    tokio::spawn(async move { a.run(&client_a).await });

    let client_b: DocSyncClient = local.establish().await.expect("client b");
    let mut b = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
    let doc_b = b.doc().clone();
    tokio::spawn(async move { b.run(&client_b).await });

    // A writes — B and the canonical doc must see it.
    doc_a
        .loro()
        .get_map("root")
        .insert("from-a", "hello")
        .expect("insert a");
    doc_a.loro().commit();
    converge(&doc_b, "b sees a's write", |d| {
        map_value(d, "from-a").as_deref() == Some("hello")
    })
    .await;
    converge(&canonical, "canonical sees a's write", |d| {
        map_value(d, "from-a").as_deref() == Some("hello")
    })
    .await;

    // B writes concurrently-ish — A must see it.
    doc_b
        .loro()
        .get_map("root")
        .insert("from-b", "world")
        .expect("insert b");
    doc_b.loro().commit();
    converge(&doc_a, "a sees b's write", |d| {
        map_value(d, "from-b").as_deref() == Some("world")
    })
    .await;

    // A late joiner with an empty doc catches the whole history up via
    // version-vector delta (the same path a reconnect takes).
    let client_c: DocSyncClient = local.establish().await.expect("client c");
    let mut c = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
    let doc_c = c.doc().clone();
    tokio::spawn(async move { c.run(&client_c).await });
    converge(&doc_c, "late joiner has full history", |d| {
        map_value(d, "from-a").as_deref() == Some("hello")
            && map_value(d, "from-b").as_deref() == Some("world")
    })
    .await;

    scope.close().await;
}

/// A replica that edited while offline and then RESTARTED (its
/// in-memory outbox is gone, but its doc isn't) must still push its
/// history to the server: `sync` returns the server's version vector
/// and the client uploads the exact missing delta.
#[tokio::test(flavor = "multi_thread")]
async fn offline_history_pushes_back_after_restart() {
    let doc_id = Uuid::new_v4();
    let scope = Scope::new();

    let canonical = CrdtDoc::ephemeral();
    let host = DocSyncHost::new(doc_id, canonical.clone());
    let router = LayerRouter::new().with(
        doc_sync_service_descriptor(),
        crdt::sync::DocSyncDispatcher::new(host),
    );
    let local = LocalServer::serve(router, scope.clone());

    // "Offline edits, then restart": the doc has history that predates
    // the SyncedDoc wiring, so nothing is in the outbox.
    let replica = CrdtDoc::ephemeral();
    replica
        .loro()
        .get_map("root")
        .insert("offline", "edit")
        .expect("insert");
    replica.loro().commit();

    let client: DocSyncClient = local.establish().await.expect("client");
    let mut synced = SyncedDoc::new(doc_id, replica);
    tokio::spawn(async move { synced.run(&client).await });

    converge(&canonical, "server received the offline history", |d| {
        map_value(d, "offline").as_deref() == Some("edit")
    })
    .await;
    scope.close().await;
}

/// Minimal probe: a single replica attaches, the first down frame is
/// `Attached` with a decodable server version vector, the call stays in
/// flight for the session (vox 0.10 channel scoping — the call IS the
/// session), and closing the up channel ends it cleanly.
#[tokio::test(flavor = "multi_thread")]
async fn single_replica_attach_returns() {
    use crdt::sync::SyncDown;

    let doc_id = Uuid::new_v4();
    let scope = Scope::new();
    let canonical = CrdtDoc::ephemeral();
    let host = DocSyncHost::new(doc_id, canonical.clone());
    let router = LayerRouter::new().with(
        doc_sync_service_descriptor(),
        crdt::sync::DocSyncDispatcher::new(host),
    );
    let local = LocalServer::serve(router, scope.clone());

    let client: DocSyncClient = local.establish().await.expect("client");
    let (up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, mut down_rx) = vox::channel::<SyncDown>();
    let mut call = std::pin::pin!(client.sync(doc_id, Vec::new(), up_rx, down_tx));

    // The Attached frame arrives while the call is still in flight.
    let mut attached: Option<SyncDown> = None;
    tokio::select! {
        res = &mut call => panic!("sync call ended before the session: {res:?}"),
        frame = tokio::time::timeout(Duration::from_secs(5), down_rx.recv()) => {
            let frame = frame.expect("attach frame timed out").expect("down channel error");
            let frame = frame.expect("down channel closed before attach");
            let _ = frame.map(|f| attached = Some(f.clone()));
        }
    }
    match attached {
        Some(SyncDown::Attached { server_vv, backlog }) => {
            assert!(loro::VersionVector::decode(&server_vv).is_ok());
            // A fresh doc's backlog is a valid (possibly header-only)
            // Loro export — it must import cleanly.
            let probe = loro::LoroDoc::new();
            probe.import(&backlog).expect("backlog imports cleanly");
        }
        other => panic!("expected Attached first, got {other:?}"),
    }

    // Closing the up channel ends the session; the held call resolves.
    drop(up_tx);
    tokio::time::timeout(Duration::from_secs(5), call)
        .await
        .expect("session end timed out")
        .expect("sync session errored");
    scope.close().await;
}

/// Shutdown regression guard: a host + established client must tear
/// down promptly. This once deadlocked — the compaction bridge closure
/// owned a strong `CrdtDoc`, so the doc's final drop happened inside
/// loro's unsubscribe path (subscriber-registry lock held) and dropping
/// the doc's own subscription re-entered the lock. Subscription
/// closures hold `WeakCrdtDoc` only; see `CrdtDoc::downgrade`.
#[tokio::test(flavor = "multi_thread")]
async fn establish_then_close() {
    let doc_id = Uuid::new_v4();
    let scope = Scope::new();
    let host = DocSyncHost::new(doc_id, CrdtDoc::ephemeral());
    let router = LayerRouter::new().with(
        doc_sync_service_descriptor(),
        crdt::sync::DocSyncDispatcher::new(host),
    );
    let local = LocalServer::serve(router, scope.clone());
    let _client: DocSyncClient = local.establish().await.expect("client");
    scope.close().await;
}
