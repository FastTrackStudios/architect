//! `crdt::docs` end to end over the in-process vox transport: a host
//! (`DocsHost` over an in-memory catalog and registry) and devices
//! (`DocsDriver`s), each device going offline by dropping its session.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
use crdt::docs::{
    CreateDoc, DocChange, DocMeta, DocQuery, DocStorage, DocsClient, DocsDriver, DocsError,
    DocsHost, DocsLane, DocsStreamClient, InMemoryDocsIndex, InMemoryDocsStore, SharedStorage,
    TursoDocsIndex, docs_rpc_service_descriptor, serve, stream_layer,
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
