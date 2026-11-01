//! `crdt::docs::OrgSyncs` end to end: a device's driver for one org, kept
//! connected to the org's server through a dropped and restored
//! connection, its documents in document files; and the server checking
//! the org's permissions on every call, so a person who is not a member
//! is refused.
//!
//! The server is a `DocsHost` (catalog + content) behind a real
//! `PermissionsGate` whose resolver presents the dialing person, over
//! in-process vox links the test can cut. The gate lets every call in
//! (`UnlistedPolicy::Allow`): what refuses a non-member here is the
//! host's and the registry's own check against the org's engine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::significant_drop_tightening
)]
#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::{PermissionsGate, StaticPrincipal, UnlistedPolicy};
use architect_orgs::{Org, Orgs, Role};
use architect_permissions::{AllowAll, PermissionEngine, Principal, RoleEngine};
use crdt::docs::{
    CreateDoc, Dialed, DocPermissions, DocQuery, DocsClient, DocsError, DocsHost, DocsLane,
    InMemoryDocsIndex, InMemoryDocsStore, Lineage, OrgDocs, OrgLink, OrgSyncs, Placement,
    docs_rpc_service_descriptor, serve, stream_layer,
};
use crdt::file::{DiskStorage, DocFile, MAIN};
use crdt::in_memory::InMemoryPersistence;
use crdt::sync::{DocSyncClient, DocSyncDispatcher, SyncDown, doc_sync_service_descriptor};
use crdt::{CrdtDoc, DocKind, DocKinds, DocRegistry};
use uuid::Uuid;

const BODY: &str = "body";
const NOTE: &str = "test.note";

fn kind() -> DocKind {
    DocKind::new(NOTE, "Note").extension("note")
}

fn kinds() -> DocKinds {
    let mut kinds = DocKinds::new();
    kinds.register(kind());
    kinds
}

fn org() -> Org {
    Org::new(
        "4f1c2b9e-8d3a-4c5e-9f00-000000000094",
        "Fast Track Audio",
        Role::Member,
    )
}

/// The org's engine: ann is a member, nobody else is.
fn org_engine() -> Arc<dyn PermissionEngine> {
    let mut engine = RoleEngine::new();
    engine.set_member("ann", "member");
    Arc::new(engine)
}

fn user(id: &str) -> Principal {
    Principal::User {
        user_id: id.to_owned(),
    }
}

/// The org's server: its catalog and the content of its docs, both
/// checking the org's permissions.
async fn server(org: &Org) -> (DocsHost, DocRegistry) {
    let permissions = DocPermissions::new(org_engine()).in_org(&org.id);
    let bytes = InMemoryPersistence::new();
    let registry = DocRegistry::new(move |id| {
        let bytes = bytes.clone();
        Box::pin(async move { CrdtDoc::open(id, bytes).await })
    })
    .with_permissions(permissions.clone());
    let host = DocsHost::open(InMemoryDocsStore::new(), registry.clone())
        .await
        .unwrap()
        .with_permissions(permissions);
    (host, registry)
}

/// The server's connections, and whether it is reachable at all.
#[derive(Clone)]
struct Wire {
    router: LayerRouter,
    up: Arc<AtomicBool>,
    /// The server's side of every connection, to cut them.
    links: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    held: Arc<Mutex<Vec<vox::ConnectionHandle>>>,
}

impl Wire {
    fn new(host: &DocsHost) -> Self {
        let router = LayerRouter::new()
            .with(docs_rpc_service_descriptor(), serve(host.clone()))
            .merge(stream_layer(host.clone()))
            .with(
                doc_sync_service_descriptor(),
                DocSyncDispatcher::new(host.clone()),
            );
        Self {
            router,
            up: Arc::new(AtomicBool::new(true)),
            links: Arc::new(Mutex::new(Vec::new())),
            held: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// One client on its own connection, as `who`.
    async fn client<C: vox::FromVoxLane>(&self, who: &Principal) -> C {
        let gated =
            PermissionsGate::new(Arc::new(AllowAll), Arc::new(StaticPrincipal(who.clone())))
                .unlisted(UnlistedPolicy::Allow)
                .wrap(self.router.clone());
        let (client_link, server_link) = vox::memory_link_pair(64);
        let held = self.held.clone();
        let server = tokio::spawn(async move {
            if let Ok(connection) = vox::acceptor_on(server_link)
                .on_lane(handler_acceptor(gated))
                .establish_connection()
                .await
            {
                held.lock().unwrap().push(connection.clone());
                let _hold = connection;
                std::future::pending::<()>().await;
            }
        });
        self.links.lock().unwrap().push(server);
        vox::initiator_on(client_link)
            .establish::<C>()
            .await
            .expect("establish")
    }

    /// The org's lane, as `who` — unless the server is down.
    async fn dial(&self, who: &Principal) -> Dialed {
        if !self.up.load(Ordering::SeqCst) {
            return Dialed::Unreachable("the server is down".into());
        }
        Dialed::Lane(DocsLane {
            docs: self.client(who).await,
            changes: self.client(who).await,
            sync: self.client(who).await,
        })
    }

    /// The org's lane as `who`, its three clients over one connection
    /// (an app's one supervised connection).
    async fn dial_one(&self, who: &Principal) -> Dialed {
        let raw: architect::local::RawLaneCaller = self.client(who).await;
        let mut lane = DocsLane::over(&raw.caller);
        // The connection is the dialer's to keep open.
        lane.docs.connection = raw.connection;
        Dialed::Lane(lane)
    }

    /// The server goes away: every connection cut, no new ones.
    fn down(&self) {
        self.up.store(false, Ordering::SeqCst);
        for connection in self.held.lock().unwrap().drain(..) {
            let _ = connection.shutdown();
        }
        for link in self.links.lock().unwrap().drain(..) {
            link.abort();
        }
    }

    fn up(&self) {
        self.up.store(true, Ordering::SeqCst);
    }
}

/// A device signed in as `who`: one driver per org, its docs under `dir`.
fn device(wire: &Wire, who: Principal, dir: &Path) -> OrgSyncs {
    device_dialing(wire, who, dir, false)
}

/// [`device`], its lane over one connection when `one`.
fn device_dialing(wire: &Wire, who: Principal, dir: &Path, one: bool) -> OrgSyncs {
    let dir = dir.to_owned();
    let wire = wire.clone();
    OrgSyncs::new(
        move |org: Org| {
            let placement = Placement::new(dir.join(&org.slug), kinds());
            async move { Ok(OrgDocs::new(org, InMemoryDocsIndex::new(), placement)) }
        },
        move |_org: Org| {
            let (wire, who) = (wire.clone(), who.clone());
            async move {
                if one {
                    wire.dial_one(&who).await
                } else {
                    wire.dial(&who).await
                }
            }
        },
    )
}

fn note(name: &str) -> CreateDoc {
    CreateDoc {
        doc_id: Uuid::new_v4(),
        kind: NOTE.into(),
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
async fn a_members_doc_syncs_through_a_dropped_connection_and_merges_offline_edits() {
    let org = org();
    let (host, registry) = server(&org).await;
    let wire = Wire::new(&host);
    let dir = tempfile::tempdir().unwrap();
    let syncs = device(&wire, user("ann"), dir.path());

    // The current org's driver, connected.
    let docs = syncs
        .current(&Orgs::new(vec![org.clone()]))
        .await
        .unwrap()
        .expect("one org, current");
    assert_eq!(docs.org().id, org.id);
    eventually("connected", async || docs.link() == OrgLink::Live).await;

    // A note made and written online reaches the server.
    let req = note("Setlist");
    let id = req.doc_id;
    let mine = docs.driver().create(req).await.unwrap();
    write(&mine, "online ");
    eventually("the server holds the note", async || {
        host.admits(id) && registry.doc(id).await.is_ok_and(|d| read(&d) == "online ")
    })
    .await;

    // The connection drops. The device goes on editing, and so does the
    // server's copy (another member, say).
    wire.down();
    eventually("offline", async || docs.link() != OrgLink::Live).await;
    write(&mine, "offline ");
    write(&registry.doc(id).await.unwrap(), "server ");
    assert!(!read(&mine).contains("server "), "nothing arrives offline");

    // Back: the reconnect loop dials again by itself, and both sides end
    // with both edits.
    wire.up();
    eventually("the two copies merge", async || {
        let theirs = read(&registry.doc(id).await.unwrap());
        let ours = read(&mine);
        ours == theirs && ours.contains("offline ") && ours.contains("server ")
    })
    .await;
    assert!(read(&mine).starts_with("online "));
    assert_eq!(docs.link(), OrgLink::Live);

    // Everything is in the note's own file, placed by the policy:
    // `<org>/<id>.note`, each step durable there.
    let path = dir.path().join(&org.slug).join(format!("{id}.note"));
    let file = DocFile::open(DiskStorage::open(&path).unwrap(), &kind())
        .await
        .unwrap();
    assert_eq!(file.id().await, Some(id));
    let main = file.open_branch(MAIN).await.unwrap();
    let kept = main.loro().get_text(BODY).to_string();
    assert!(
        kept.contains("online ") && kept.contains("offline ") && kept.contains("server "),
        "the file holds the merged note: {kept:?}"
    );

    // Leaving the org stops its sync.
    syncs.follow(&Orgs::new(Vec::new())).await.unwrap();
    assert_eq!(docs.link(), OrgLink::Stopped);
    assert!(syncs.orgs().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_doc_made_on_another_device_lands_in_a_file_placed_by_the_app() {
    let org = org();
    let (host, _registry) = server(&org).await;
    let wire = Wire::new(&host);
    let (laptop_dir, phone_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let laptop = device(&wire, user("ann"), laptop_dir.path());
    let phone = device_dialing(&wire, user("ann"), phone_dir.path(), true);

    let on_laptop = laptop.ensure(&org).await.unwrap();
    let req = note("Rig");
    let id = req.doc_id;
    let made = on_laptop.driver().create(req).await.unwrap();
    write(&made, "from the laptop");

    // The phone keeps this one where its app keeps rigs.
    let on_phone = phone.ensure(&org).await.unwrap();
    let placed = phone_dir.path().join("rigs").join("Rig.note");
    on_phone.placement().place(id, &placed);
    eventually("the phone holds the laptop's note", async || {
        on_phone
            .driver()
            .open(id)
            .await
            .is_ok_and(|d| read(&d) == "from the laptop")
    })
    .await;
    let file = DocFile::open(DiskStorage::open(&placed).unwrap(), &kind())
        .await
        .unwrap();
    assert_eq!(file.id().await, Some(id), "the placed file is the note");
    assert!(on_phone.placement().file(id).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_who_is_not_a_member_is_refused_on_the_server() {
    let org = org();
    let (host, registry) = server(&org).await;
    let wire = Wire::new(&host);

    // A member registers a note, so there is something to be refused.
    let ann_docs: DocsClient = wire.client(&user("ann")).await;
    let req = note("Private");
    let id = req.doc_id;
    ann_docs.create(req).await.unwrap();
    write(&registry.doc(id).await.unwrap(), "members only");

    // Bob signs in fine (the gate lets him in), but is not in the org.
    let bob = user("bob");
    let docs: DocsClient = wire.client(&bob).await;
    let listed = docs.list(DocQuery::default()).await.unwrap();
    assert!(listed.docs.is_empty(), "he sees nothing listed");
    let got = docs.get(id).await.unwrap_err();
    assert!(
        matches!(got, vox::VoxError::User(ref e) if **e == DocsError::Forbidden),
        "{got:?}"
    );
    let made = docs.create(note("Mine")).await.unwrap_err();
    assert!(
        matches!(made, vox::VoxError::User(ref e) if **e == DocsError::Forbidden),
        "{made:?}"
    );

    // His sync of the note is refused before anything is opened or sent.
    let sync: DocSyncClient = wire.client(&bob).await;
    let (_up_tx, up_rx) = vox::channel::<Vec<u8>>();
    let (down_tx, mut down_rx) = vox::channel::<SyncDown>();
    let refused = tokio::time::timeout(
        Duration::from_secs(5),
        sync.sync(id, Vec::new(), up_rx, down_tx),
    )
    .await
    .expect("answered in time")
    .unwrap_err();
    assert!(format!("{refused:?}").contains("UnknownDoc"), "{refused:?}");
    let came = tokio::time::timeout(Duration::from_millis(300), down_rx.recv()).await;
    assert!(
        !matches!(came, Ok(Ok(Some(_)))),
        "none of the note came down"
    );

    // And so is a driver of his: it connects, and syncs nothing.
    let dir = tempfile::tempdir().unwrap();
    let syncs = device(&wire, bob, dir.path());
    let his = syncs.ensure(&org).await.unwrap();
    eventually("connected", async || his.link() == OrgLink::Live).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(his.driver().docs().await.unwrap().is_empty());
}
