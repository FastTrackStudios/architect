//! `crdt::docs` end to end over the in-process vox transport: a host
//! (`DocsHost` over an in-memory catalog and registry) and devices
//! (`DocsDriver`s), each device going offline by dropping its session.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
use crdt::docs::{
    CreateDoc, DocChange, DocMeta, DocQuery, DocStorage, DocSyncEvent, DocsClient, DocsDriver,
    DocsError, DocsHost, DocsLane, DocsStreamClient, DriverHost, InMemoryDocsIndex,
    InMemoryDocsStore, Lineage, Refusal, SharedStorage, TursoDocsIndex,
    docs_rpc_service_descriptor, serve, stream_layer,
};
use crdt::in_memory::InMemoryPersistence;
use crdt::sync::{DocSyncClient, DocSyncDispatcher, SyncDown, doc_sync_service_descriptor};
use crdt::{CrdtDoc, DocRegistry, PersistError, Persistence, TursoPersistence};
use uuid::Uuid;

const BODY: &str = "body";

async fn host() -> DocsHost {
    let bytes = InMemoryPersistence::new();
    let registry = DocRegistry::new(move |id| {
        let bytes = bytes.clone();
        Box::pin(async move { CrdtDoc::open(id, bytes).await })
    });
    DocsHost::open(InMemoryDocsStore::new(), registry)
        .await
        .unwrap()
}

fn router(host: &DocsHost) -> LayerRouter {
    LayerRouter::new()
        .with(docs_rpc_service_descriptor(), serve(host.clone()))
        .merge(stream_layer(host.clone()))
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(host.clone()),
        )
}

async fn lane(local: &LocalServer) -> DocsLane {
    DocsLane {
        docs: local.establish::<DocsClient>().await.unwrap(),
        changes: local.establish::<DocsStreamClient>().await.unwrap(),
        sync: local.establish::<DocSyncClient>().await.unwrap(),
    }
}

/// A device online: its session runs until the handle is aborted.
fn online(driver: &DocsDriver, lane: DocsLane) -> tokio::task::JoinHandle<()> {
    let driver = driver.clone();
    tokio::spawn(async move {
        let _ = driver.run(&lane).await;
    })
}

fn memory_device() -> DocsDriver {
    DocsDriver::new(
        InMemoryDocsIndex::new(),
        SharedStorage(Arc::new(InMemoryPersistence::new())),
    )
}

fn session(name: &str) -> CreateDoc {
    CreateDoc {
        doc_id: Uuid::new_v4(),
        kind: "session".into(),
        name: name.into(),
        parent: None,
        lineage: Lineage::Strict,
    }
}

fn write(doc: &CrdtDoc, at_end: &str) {
    let text = doc.loro().get_text(BODY);
    text.insert(text.len_unicode(), at_end).unwrap();
    doc.loro().commit();
}

fn read(doc: &CrdtDoc) -> String {
    doc.loro().get_text(BODY).to_string()
}

async fn eventually(what: &str, mut cond: impl AsyncFnMut() -> bool) {
    for _ in 0..400 {
        if cond().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_devices_edit_apart_and_converge() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let (a, b) = (memory_device(), memory_device());

    // Made on the laptop with no connection, and written to.
    let req = session("Gig");
    let id = req.doc_id;
    let on_a = a.create(req).await.unwrap();
    write(&on_a, "intro ");
    assert!(!host.admits(id), "nothing reached the host yet");

    // Both come online: the laptop registers and pushes, the phone adopts.
    let mut b_changes = b.changes();
    let run_a = online(&a, lane(&local).await);
    let run_b = online(&b, lane(&local).await);
    eventually("the phone holds the laptop's doc", async || {
        b.open(id).await.is_ok_and(|d| read(&d) == "intro ")
    })
    .await;
    let mut adopted = None;
    while let Ok(change) = b_changes.try_recv() {
        if let DocChange::Created(meta) = change {
            adopted = Some(meta);
        }
    }
    let adopted = adopted.expect("the phone was told of the new doc");
    assert_eq!(
        (adopted.kind.as_str(), adopted.name.as_str()),
        ("session", "Gig")
    );
    assert!(adopted.rev > 0, "the phone holds the host's row");

    // Both offline; each edits.
    run_a.abort();
    run_b.abort();
    let _ = (run_a.await, run_b.await);
    let on_b = b.open(id).await.unwrap();
    write(&on_a, "laptop ");
    write(&on_b, "phone ");

    // Back online: one document, both edits.
    let run_a = online(&a, lane(&local).await);
    let run_b = online(&b, lane(&local).await);
    eventually("the two copies converge", async || {
        let (x, y) = (read(&on_a), read(&on_b));
        x == y && x.contains("laptop ") && x.contains("phone ")
    })
    .await;
    assert!(read(&on_a).starts_with("intro "));
    run_a.abort();
    run_b.abort();
    scope.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_catalog_streams_created_renamed_and_deleted() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let docs: DocsClient = local.establish().await.unwrap();
    let stream: DocsStreamClient = local.establish().await.unwrap();
    let (tx, mut rx) = vox::channel::<DocChange>();
    let _sub = tokio::spawn(async move {
        let _ = stream.changes(tx).await;
    });
    let mut next = async || {
        let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a frame in time")
            .expect("rx error")
            .expect("rx open");
        let mut owned = None;
        let _ = frame.map(|f| owned = Some(f));
        owned.expect("a frame")
    };
    assert_eq!(next().await, DocChange::Attached { head_rev: 0 });

    let req = session("Setlist A");
    let id = req.doc_id;
    let made = docs.create(req.clone()).await.unwrap();
    assert_eq!(made.rev, 1);
    assert_eq!(next().await, DocChange::Created(made.clone()));
    let again = docs
        .create(CreateDoc {
            name: "another name".into(),
            ..req.clone()
        })
        .await
        .unwrap();
    assert_eq!(again, made, "create is idempotent: the first row stands");
    let clash = docs
        .create(CreateDoc {
            kind: "rig".into(),
            ..req.clone()
        })
        .await
        .unwrap_err();
    assert!(
        matches!(clash, vox::VoxError::User(ref e) if **e == DocsError::KindMismatch("session".into())),
        "{clash:?}"
    );

    let renamed = docs.rename(id, "Setlist B".into(), Some(1)).await.unwrap();
    assert_eq!((renamed.rev, renamed.name.as_str()), (2, "Setlist B"));
    assert_eq!(next().await, DocChange::Renamed(renamed.clone()));
    let stale = docs.rename(id, "C".into(), Some(1)).await.unwrap_err();
    assert!(matches!(stale, vox::VoxError::User(ref e) if **e == DocsError::Stale(2)));

    let gone = docs.delete(id).await.unwrap();
    assert!(gone.deleted);
    assert_eq!(next().await, DocChange::Deleted(gone.clone()));
    assert_eq!(docs.delete(id).await.unwrap(), gone, "delete is idempotent");
    let back = docs.create(req).await.unwrap_err();
    assert!(matches!(back, vox::VoxError::User(ref e) if **e == DocsError::Deleted));

    // The tombstone is listed for whoever asks for tombstones.
    let live = docs.list(DocQuery::default()).await.unwrap();
    assert!(live.docs.is_empty());
    assert_eq!(live.cursor, 3);
    let all = docs
        .list(DocQuery {
            include_deleted: true,
            ..DocQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(all.docs, vec![gone]);

    // And its content is no longer served.
    assert!(!host.admits(id));
    let sync: DocSyncClient = local.establish().await.unwrap();
    let (_up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, _down_rx) = vox::channel::<SyncDown>();
    let refused = sync.sync(id, Vec::new(), up_rx, down_tx).await;
    assert!(refused.is_err(), "a deleted doc does not sync");
    scope.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offline_device_learns_of_a_delete() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let (a, b) = (memory_device(), memory_device());
    let req = session("Old mix");
    let id = req.doc_id;
    a.create(req).await.unwrap();
    let run_a = online(&a, lane(&local).await);
    let run_b = online(&b, lane(&local).await);
    eventually("the phone holds it", async || b.open(id).await.is_ok()).await;
    run_b.abort();
    let _ = run_b.await;

    // Deleted while the phone is away.
    let docs: DocsClient = local.establish().await.unwrap();
    docs.delete(id).await.unwrap();

    let run_b = online(&b, lane(&local).await);
    eventually("the phone learns of the delete", async || {
        b.docs()
            .await
            .unwrap()
            .iter()
            .any(|d| d.meta.doc_id == id && d.meta.deleted)
    })
    .await;
    assert!(matches!(b.open(id).await, Err(DocsError::Deleted)));
    run_a.abort();
    run_b.abort();
    scope.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_received_edit_survives_a_restart_offline() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("phone.db");
    let db = db.to_str().unwrap().to_owned();
    let phone = async || {
        DocsDriver::new(
            TursoDocsIndex::open_path(&db).await.unwrap(),
            SharedStorage(Arc::new(TursoPersistence::open_path(&db).await.unwrap())),
        )
    };

    let laptop = memory_device();
    let req = session("Charts");
    let id = req.doc_id;
    let on_laptop = laptop.create(req).await.unwrap();
    write(&on_laptop, "from the laptop");
    let run_laptop = online(&laptop, lane(&local).await);

    {
        let b = phone().await;
        let run_b = online(&b, lane(&local).await);
        eventually("the phone received the edit", async || {
            b.open(id)
                .await
                .is_ok_and(|d| read(&d) == "from the laptop")
        })
        .await;
        run_b.abort();
        let _ = run_b.await;
    }
    run_laptop.abort();
    scope.close().await;

    // A new process, no connection: the edit is on the phone's disk.
    let b = phone().await;
    let doc = b.open(id).await.unwrap();
    assert_eq!(read(&doc), "from the laptop");
}

/// Records the order docs are asked for storage.
#[derive(Clone, Default)]
struct Recording {
    asked: Arc<Mutex<Vec<Uuid>>>,
    bytes: InMemoryPersistence,
}

#[async_trait::async_trait]
impl DocStorage for Recording {
    async fn persistence(&self, meta: &DocMeta) -> Result<Arc<dyn Persistence>, PersistError> {
        self.asked.lock().unwrap().push(meta.doc_id);
        Ok(Arc::new(self.bytes.clone()))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branches_made_offline_register_and_arrive_parent_first() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let a = memory_device();
    let project = session("Album");
    let branch = CreateDoc {
        parent: Some(project.doc_id),
        ..session("Album (alt mix)")
    };
    a.create(project.clone()).await.unwrap();
    a.create(branch.clone()).await.unwrap();
    // The parent renamed after the fork: by revision it now comes last.
    let run_a = online(&a, lane(&local).await);
    eventually("both registered", async || {
        host.admits(project.doc_id) && host.admits(branch.doc_id)
    })
    .await;
    let docs: DocsClient = local.establish().await.unwrap();
    docs.rename(project.doc_id, "Album (final)".into(), None)
        .await
        .unwrap();
    let rows = docs.list(DocQuery::default()).await.unwrap().docs;
    assert_eq!(
        rows.iter()
            .find(|d| d.doc_id == branch.doc_id)
            .unwrap()
            .parent,
        Some(project.doc_id),
        "the parent is carried"
    );

    let storage = Recording::default();
    let b = DocsDriver::new(InMemoryDocsIndex::new(), storage.clone());
    let run_b = online(&b, lane(&local).await);
    eventually("the new device holds both", async || {
        storage.asked.lock().unwrap().len() == 2
    })
    .await;
    assert_eq!(
        *storage.asked.lock().unwrap(),
        vec![project.doc_id, branch.doc_id],
        "a branch's storage is made after its parent's"
    );
    run_a.abort();
    run_b.abort();
    scope.close().await;
}

/// A store whose first `append_update` fails.
#[derive(Clone, Default)]
struct FailsFirstWrite {
    inner: InMemoryPersistence,
    failed: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl Persistence for FailsFirstWrite {
    async fn load_snapshot(&self, doc_id: Uuid) -> Result<Option<Vec<u8>>, PersistError> {
        self.inner.load_snapshot(doc_id).await
    }
    async fn write_snapshot(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        self.inner.write_snapshot(doc_id, bytes).await
    }
    async fn append_update(&self, doc_id: Uuid, bytes: &[u8]) -> Result<(), PersistError> {
        if !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(PersistError::Backend("disk full".into()));
        }
        self.inner.append_update(doc_id, bytes).await
    }
    async fn load_updates(&self, doc_id: Uuid) -> Result<Vec<Vec<u8>>, PersistError> {
        self.inner.load_updates(doc_id).await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_received_edit_whose_write_failed_is_written_on_the_retry() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let laptop = memory_device();
    let req = session("Takes");
    let id = req.doc_id;
    write(&laptop.create(req).await.unwrap(), "take three");
    let run_laptop = online(&laptop, lane(&local).await);
    eventually("registered", async || host.admits(id)).await;

    // The phone's first write of what it receives fails; the session
    // ends, and the retry writes it before attaching again.
    let disk = FailsFirstWrite::default();
    let index = InMemoryDocsIndex::new();
    let phone = DocsDriver::new(index.clone(), SharedStorage(Arc::new(disk.clone())));
    let run_phone = online(&phone, lane(&local).await);
    eventually("the edit is on the phone's disk", async || {
        disk.inner
            .load_updates(id)
            .await
            .is_ok_and(|u| !u.is_empty())
    })
    .await;
    run_phone.abort();
    let _ = run_phone.await;
    run_laptop.abort();
    scope.close().await;

    // Restarted offline: the edit is there.
    let reopened = DocsDriver::new(index, SharedStorage(Arc::new(disk.inner.clone())));
    assert_eq!(read(&reopened.open(id).await.unwrap()), "take three");
}

// ── Lineage, sync events, peers ────────────────────────────────────────

/// One root map, entries by key: the shape that merges per field.
const LIBRARY: &str = "library";

fn put_entry(doc: &CrdtDoc, key: &str, value: &str) {
    doc.loro().get_map(LIBRARY).insert(key, value).unwrap();
    doc.loro().commit();
}

fn entry(doc: &CrdtDoc, key: &str) -> Option<String> {
    doc.loro()
        .get_map(LIBRARY)
        .get(key)
        .and_then(|v| v.into_value().ok())
        .and_then(|v| v.into_string().ok())
        .map(|s| s.to_string())
}

fn rig(doc_id: Uuid, lineage: Lineage) -> CreateDoc {
    CreateDoc {
        doc_id,
        kind: "rig".into(),
        name: "Bass rig".into(),
        parent: None,
        lineage,
    }
}

/// Every sync event a device has seen so far.
fn seen(rx: &mut tokio::sync::broadcast::Receiver<DocSyncEvent>, log: &mut Vec<DocSyncEvent>) {
    while let Ok(ev) = rx.try_recv() {
        log.push(ev);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merge_doc_made_on_two_devices_offline_converges_with_both_edits() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let (a, b) = (memory_device(), memory_device());
    let (mut a_events, mut b_events) = (a.sync_events(), b.sync_events());

    // One derived id, made on both with nothing attached, each editing
    // its own entry.
    let id = Uuid::new_v4();
    let on_a = a.create(rig(id, Lineage::Merge)).await.unwrap();
    put_entry(&on_a, "amp", "from the laptop");
    let on_b = b.create(rig(id, Lineage::Merge)).await.unwrap();
    put_entry(&on_b, "cab", "from the phone");

    let run_a = online(&a, lane(&local).await);
    let run_b = online(&b, lane(&local).await);
    eventually("both copies hold both entries", async || {
        [&on_a, &on_b].iter().all(|d| {
            entry(d, "amp").as_deref() == Some("from the laptop")
                && entry(d, "cab").as_deref() == Some("from the phone")
        })
    })
    .await;
    let mut log = Vec::new();
    seen(&mut a_events, &mut log);
    seen(&mut b_events, &mut log);
    assert!(
        !log.iter()
            .any(|e| matches!(e, DocSyncEvent::Refused { .. })),
        "nothing refused: {log:?}"
    );
    assert!(
        log.iter()
            .any(|e| matches!(e, DocSyncEvent::Attached { doc_id, .. } if *doc_id == id)),
        "both sessions attached: {log:?}"
    );

    // The policy is the first registration's.
    let docs: DocsClient = local.establish().await.unwrap();
    let clash = docs.create(rig(id, Lineage::Strict)).await.unwrap_err();
    assert!(
        matches!(clash, vox::VoxError::User(ref e) if **e == DocsError::LineageMismatch(Lineage::Merge)),
        "{clash:?}"
    );
    run_a.abort();
    run_b.abort();
    scope.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_strict_doc_made_twice_is_refused_and_reset_takes_the_hosts() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let (a, b) = (memory_device(), memory_device());
    let id = Uuid::new_v4();
    put_entry(
        &a.create(rig(id, Lineage::Strict)).await.unwrap(),
        "amp",
        "a",
    );
    let run_a = online(&a, lane(&local).await);
    eventually("the host holds the laptop's copy", async || {
        host.registry()
            .doc(id)
            .await
            .is_ok_and(|d| entry(&d, "amp").is_some())
    })
    .await;

    // The same id made apart on the phone: another history.
    put_entry(
        &b.create(rig(id, Lineage::Strict)).await.unwrap(),
        "cab",
        "b",
    );
    let mut b_events = b.sync_events();
    let run_b = online(&b, lane(&local).await);
    let mut log = Vec::new();
    eventually("the phone's copy is refused", async || {
        seen(&mut b_events, &mut log);
        log.iter().any(|e| {
            matches!(e, DocSyncEvent::Refused { doc_id, reason: Refusal::ForeignLineage, .. } if *doc_id == id)
        })
    })
    .await;

    // Reset: the phone drops its copy and takes the host's.
    b.reset(id).await.unwrap();
    eventually("the phone holds the host's copy", async || {
        b.open(id)
            .await
            .is_ok_and(|d| entry(&d, "amp").as_deref() == Some("a") && entry(&d, "cab").is_none())
    })
    .await;
    seen(&mut b_events, &mut log);
    assert!(log.contains(&DocSyncEvent::Reset { doc_id: id }));
    run_a.abort();
    run_b.abort();
    scope.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_comes_once_the_hosts_history_is_applied() {
    let host = host().await;
    let scope = Scope::new();
    let local = LocalServer::serve(router(&host), scope.clone());
    let (a, b) = (memory_device(), memory_device());
    let req = session("Setlist");
    let id = req.doc_id;
    write(&a.create(req).await.unwrap(), "opener");
    let run_a = online(&a, lane(&local).await);
    eventually("registered", async || host.admits(id)).await;
    eventually("the host holds the text", async || {
        host.registry()
            .doc(id)
            .await
            .is_ok_and(|d| read(&d) == "opener")
    })
    .await;

    let mut events = b.sync_events();
    let run_b = online(&b, lane(&local).await);
    let attached = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(DocSyncEvent::Attached { doc_id, lane }) = events.recv().await {
                if doc_id == id {
                    return lane;
                }
            }
        }
    })
    .await
    .expect("attached in time");
    assert_eq!(attached, crdt::docs::HUB);
    assert_eq!(
        read(&b.open(id).await.unwrap()),
        "opener",
        "the backlog is applied when Attached arrives"
    );
    run_a.abort();
    run_b.abort();
    scope.close().await;
}

fn device_router(host: &DriverHost) -> LayerRouter {
    LayerRouter::new()
        .with(docs_rpc_service_descriptor(), serve(host.clone()))
        .merge(stream_layer(host.clone()))
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(host.clone()),
        )
}

/// A device's session with another device, until the handle is aborted.
fn online_peer(driver: &DocsDriver, key: &str, lane: DocsLane) -> tokio::task::JoinHandle<()> {
    let (driver, key) = (driver.clone(), key.to_owned());
    tokio::spawn(async move {
        let _ = driver.run_peer(&key, &lane).await;
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_sync_each_other_while_the_hub_is_down_and_the_hub_catches_up() {
    let task = host().await;
    let scope = Scope::new();
    let task_lanes = LocalServer::serve(router(&task), scope.clone());
    let (a, b, c) = (memory_device(), memory_device(), memory_device());

    // The phone (B) serves its docs to the laptop (A).
    let b_host = b.host(|_| true).await.unwrap();
    let b_lanes = LocalServer::serve(device_router(&b_host), scope.clone());

    // A project made on the laptop, everyone on the hub.
    let req = session("Album");
    let id = req.doc_id;
    let on_a = a.create(req).await.unwrap();
    write(&on_a, "a1 ");
    let a_hub = online(&a, lane(&task_lanes).await);
    let b_hub = online(&b, lane(&task_lanes).await);
    let c_hub = online(&c, lane(&task_lanes).await);
    eventually("the phone and the desktop hold it", async || {
        b.open(id).await.is_ok_and(|d| read(&d) == "a1 ")
            && c.open(id).await.is_ok_and(|d| read(&d) == "a1 ")
    })
    .await;
    let on_b = b.open(id).await.unwrap();
    let on_c = c.open(id).await.unwrap();

    // The laptop also dials the phone directly.
    let a_peer = online_peer(&a, "phone", lane(&b_lanes).await);

    // The hub goes away for both. An edit on the laptop reaches the
    // phone over the peer lane alone.
    a_hub.abort();
    b_hub.abort();
    let _ = (a_hub.await, b_hub.await);
    write(&on_a, "a2 ");
    eventually(
        "the phone gets the laptop's edit peer to peer",
        async || read(&on_b).contains("a2 "),
    )
    .await;
    assert!(
        !read(&on_c).contains("a2 "),
        "the hub has not seen it yet: nobody is on it"
    );

    // A doc made on the laptop while only the peer lane is up: the
    // phone adopts it.
    let fresh = session("B-sides");
    let fresh_id = fresh.doc_id;
    write(&a.create(fresh).await.unwrap(), "demo ");
    eventually("the phone adopts the laptop's new doc", async || {
        b.open(fresh_id).await.is_ok_and(|d| read(&d) == "demo ")
    })
    .await;

    // The phone comes back to the hub: the laptop's edit and its new doc
    // reach the hub through the phone, and on to the desktop.
    let b_hub = online(&b, lane(&task_lanes).await);
    eventually(
        "the desktop gets the laptop's edit through the phone",
        async || read(&on_c).contains("a2 "),
    )
    .await;
    eventually("the hub registered the doc the phone adopted", async || {
        task.admits(fresh_id) && c.open(fresh_id).await.is_ok_and(|d| read(&d) == "demo ")
    })
    .await;

    // An edit from the hub side reaches the laptop, which is on the peer
    // lane only: the phone relays it.
    write(&on_c, "c1 ");
    eventually(
        "the laptop gets the desktop's edit through the phone",
        async || read(&on_a).contains("c1 "),
    )
    .await;

    // Everyone back: one history.
    let a_hub = online(&a, lane(&task_lanes).await);
    eventually("all three converge", async || {
        let (laptop, phone, desktop) = (read(&on_a), read(&on_b), read(&on_c));
        laptop == phone && phone == desktop && laptop.contains("a2 ") && laptop.contains("c1 ")
    })
    .await;
    for run in [a_hub, b_hub, c_hub, a_peer] {
        run.abort();
    }
    scope.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_host_refuses_a_peer_it_does_not_admit() {
    let b = memory_device();
    let id = Uuid::new_v4();
    put_entry(
        &b.create(rig(id, Lineage::Merge)).await.unwrap(),
        "amp",
        "b",
    );
    let allowed = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let gate = allowed.clone();
    let b_host = b
        .host(move |_| gate.load(std::sync::atomic::Ordering::SeqCst))
        .await
        .unwrap();
    let scope = Scope::new();
    let b_lanes = LocalServer::serve(device_router(&b_host), scope.clone());
    let docs: DocsClient = b_lanes.establish().await.unwrap();
    assert_eq!(docs.list(DocQuery::default()).await.unwrap().docs.len(), 1);

    // Taken off the account: refused on the next call, no restart.
    allowed.store(false, std::sync::atomic::Ordering::SeqCst);
    let refused = docs.list(DocQuery::default()).await.unwrap_err();
    assert!(
        matches!(refused, vox::VoxError::User(ref e) if **e == DocsError::Forbidden),
        "{refused:?}"
    );
    scope.close().await;
}
