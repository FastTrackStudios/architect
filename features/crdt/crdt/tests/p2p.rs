//! `crdt::p2p`: documents between devices and through links, over real
//! iroh endpoints, with no server anywhere.
//!
//! Every node is a `DocRegistry` served by `iroh_link::serve_peers`, one
//! `node_router` per connection, so the caller each gate sees is the
//! endpoint id the QUIC handshake proved. Devices are admitted from
//! auth's `DeviceDirectory`; a guest joins by a `fts-doc:` link.
//!
//! The endpoints bind loopback and dial each other by direct address,
//! except in `devices_on_one_network_find_each_other`, which finds the
//! other node on the local network (mDNS, `_fts._udp`) and dials it by
//! bare id: that one needs an interface that carries multicast.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::time::Duration;

use architect::iroh_link::{self, LanBrowser, LanEvent, PeerLookup, VOX_ALPN};
use architect_permissions::Principal;
use auth_proto::device::{DeviceDirectory, PeerDevice};
use crdt::CrdtDoc;
use crdt::link::{DocLink, LinkKey};
use crdt::p2p::{Admission, NodeAccess, Shares, node_router};
use crdt::peer::{DocCatalog, DocPeerClient, OpenDocs, SelfLane, pull};
use crdt::registry::DocRegistry;
use crdt::sync::{DocSyncClient, SyncedDoc};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(10);

/// One device or host: its endpoint, its registry, and who it serves.
struct Node {
    endpoint: iroh::Endpoint,
    registry: DocRegistry,
    access: NodeAccess,
    _serve: tokio::task::JoinHandle<()>,
}

impl Node {
    fn id(&self) -> String {
        self.endpoint.id().to_string()
    }

    fn addr(&self) -> iroh::EndpointAddr {
        iroh::EndpointAddr::from_parts(
            self.endpoint.id(),
            self.endpoint
                .bound_sockets()
                .into_iter()
                .map(iroh::TransportAddr::Ip),
        )
    }
}

async fn loopback() -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .alpns(vec![VOX_ALPN.to_vec()])
        .bind_addr("127.0.0.1:0")
        .expect("bind_addr")
        .bind()
        .await
        .expect("bind iroh endpoint")
}

/// Serve `endpoint`'s documents: the registry admits what `NodeAccess`
/// admits, and the node's own sweep lane (`service:self`).
fn serve(endpoint: iroh::Endpoint) -> Node {
    let access = NodeAccess::new(Admission::new(), Shares::new());
    let registry = DocRegistry::new(|_| Box::pin(async { Ok(CrdtDoc::ephemeral()) }))
        .with_authorizer({
            let access = access.clone();
            move |who, doc_id| {
                access.admits(who, doc_id)
                    || matches!(who, Some(Principal::Service { name }) if name == "self")
            }
        });
    let serve = tokio::spawn({
        let endpoint = endpoint.clone();
        let registry = registry.clone();
        let access = access.clone();
        async move {
            iroh_link::serve_peers(&endpoint, move |remote| {
                node_router(
                    registry.clone(),
                    OpenDocs(registry.clone()),
                    access.clone(),
                    &remote.to_string(),
                )
            })
            .await;
        }
    });
    Node {
        endpoint,
        registry,
        access,
        _serve: serve,
    }
}

/// Auth's device directory for one account: these nodes as its devices.
fn directory(nodes: &[(Uuid, &Node)]) -> DeviceDirectory {
    DeviceDirectory {
        account_org: None,
        devices: nodes
            .iter()
            .map(|(device_id, node)| PeerDevice {
                device_id: *device_id,
                name: "device".into(),
                platform: "linux".into(),
                endpoint_id: Some(node.id()),
                relay_url: None,
                last_seen_at: chrono::Utc::now(),
            })
            .collect(),
    }
}

/// The account's devices but `me`, as a node lets them in.
fn account_peers(account: &DeviceDirectory, me: Uuid) -> Vec<architect_devices::Peer> {
    account
        .peer_endpoints(me)
        .iter()
        .map(|endpoint| architect_devices::Peer::new(endpoint, "device", "linux"))
        .collect()
}

async fn sync_client(from: &iroh::Endpoint, to: impl Into<iroh::EndpointAddr>) -> DocSyncClient {
    let link = iroh_link::connect(from, to).await.expect("dial");
    vox::initiator_on(link)
        .establish::<DocSyncClient>()
        .await
        .expect("DocSync handshake")
}

async fn peer_client(from: &iroh::Endpoint, to: &Node) -> DocPeerClient {
    let link = iroh_link::connect(from, to.addr()).await.expect("dial");
    vox::initiator_on(link)
        .establish::<DocPeerClient>()
        .await
        .expect("DocPeer handshake")
}

/// A live replica of `doc_id` on `client`'s node, editing `doc`.
fn live(doc_id: Uuid, doc: CrdtDoc, client: DocSyncClient) -> tokio::task::JoinHandle<()> {
    let mut replica = SyncedDoc::new(doc_id, doc);
    tokio::spawn(async move {
        let _ = replica.run(&client).await;
    })
}

/// One session's outcome, for a caller expected to be refused: a
/// session still running after [`WAIT`] was let in.
async fn attach(doc_id: Uuid, client: &DocSyncClient) -> Result<(), String> {
    let mut replica = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
    tokio::time::timeout(WAIT, replica.run(client))
        .await
        .map_or(Ok(()), |res| res.map_err(|e| e.to_string()))
}

fn edit(doc: &CrdtDoc, key: &str, value: &str) {
    doc.loro().get_map("root").insert(key, value).unwrap();
    doc.loro().commit();
}

fn value(doc: &CrdtDoc, key: &str) -> Option<String> {
    doc.loro()
        .get_map("root")
        .get(key)
        .and_then(|v| v.into_value().ok())
        .and_then(|v| v.into_string().ok())
        .map(|s| s.to_string())
}

async fn sees(doc: &CrdtDoc, key: &str, want: &str, what: &str) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while value(doc, key).as_deref() != Some(want) {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_joins_a_live_document_and_edits_flow_both_ways() {
    let host = serve(loopback().await);
    let doc_id = Uuid::new_v4();
    let at_host = host.registry.doc(doc_id).await.unwrap();
    edit(&at_host, "title", "draft");

    // What the host hands over, as text, read back by the guest.
    let text = host.access.shares.share(&host.id(), doc_id).to_string();
    let link: DocLink = text.parse().unwrap();
    assert_eq!(link.endpoint, host.id());
    assert_eq!(link.doc_id, doc_id);

    // The guest is nobody's device: the link is its only way in.
    let guest = loopback().await;
    let endpoint: iroh::EndpointId = link.endpoint.parse().unwrap();
    assert_eq!(endpoint, host.endpoint.id());
    let client = sync_client(&guest, host.addr())
        .await
        .with_middleware(LinkKey::new(&link));
    let at_guest = CrdtDoc::ephemeral();
    let _session = live(link.doc_id, at_guest.clone(), client);

    sees(&at_guest, "title", "draft", "the guest gets the document").await;
    edit(&at_guest, "body", "from the guest");
    sees(
        &at_host,
        "body",
        "from the guest",
        "the host gets the guest's edit",
    )
    .await;
    edit(&at_host, "title", "final");
    sees(
        &at_guest,
        "title",
        "final",
        "the guest gets the host's edit live",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_opens_its_document_only_and_not_once_revoked() {
    let host = serve(loopback().await);
    let (doc_id, other) = (Uuid::new_v4(), Uuid::new_v4());
    edit(&host.registry.doc(other).await.unwrap(), "secret", "yes");
    let link = host.access.shares.share(&host.id(), doc_id);

    let guest = loopback().await;
    let client = sync_client(&guest, host.addr())
        .await
        .with_middleware(LinkKey::new(&link));
    assert!(
        attach(other, &client).await.is_err(),
        "a link opened a document it does not name"
    );

    // The document lane is for devices; a guest cannot list.
    let peers = peer_client(&guest, &host).await;
    assert!(
        peers.list().await.is_err(),
        "a guest listed the node's documents"
    );

    // Revoking cuts off the guest already editing through the link…
    let at_guest = CrdtDoc::ephemeral();
    edit(&host.registry.doc(doc_id).await.unwrap(), "title", "draft");
    let session = live(doc_id, at_guest.clone(), client.clone());
    sees(&at_guest, "title", "draft", "the guest joins").await;
    host.access.shares.revoke(&link);
    tokio::time::timeout(WAIT, session)
        .await
        .expect("a revoked guest's session ended")
        .unwrap();
    // …and refuses it from then on.
    assert!(
        attach(doc_id, &client).await.is_err(),
        "a revoked link still opens its document"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn registered_devices_edit_live_with_no_server() {
    let laptop = serve(loopback().await);
    let phone = serve(loopback().await);
    let (laptop_id, phone_id) = (Uuid::new_v4(), Uuid::new_v4());
    let account = directory(&[(laptop_id, &laptop), (phone_id, &phone)]);
    laptop
        .access
        .devices
        .set_account(account_peers(&account, laptop_id));
    phone
        .access
        .devices
        .set_account(account_peers(&account, phone_id));

    // Live: the phone's copy of the song stays attached to the laptop's.
    let song = Uuid::new_v4();
    let on_laptop = laptop.registry.doc(song).await.unwrap();
    let on_phone = phone.registry.doc(song).await.unwrap();
    edit(&on_laptop, "tempo", "120");
    let client = sync_client(&phone.endpoint, laptop.addr()).await;
    let _session = live(song, on_phone.clone(), client);
    sees(
        &on_phone,
        "tempo",
        "120",
        "the phone gets the laptop's song",
    )
    .await;
    edit(&on_phone, "key", "D");
    sees(&on_laptop, "key", "D", "the laptop gets the phone's edit").await;
    edit(&on_laptop, "tempo", "96");
    sees(
        &on_phone,
        "tempo",
        "96",
        "the phone gets the laptop's next edit",
    )
    .await;

    // A sweep: a note the phone has never opened comes over with the rest.
    let note = Uuid::new_v4();
    edit(&laptop.registry.doc(note).await.unwrap(), "text", "hello");
    let local = SelfLane::open(
        phone.registry.clone(),
        Principal::Service {
            name: "self".into(),
        },
    )
    .await
    .unwrap();
    let ours: HashMap<Uuid, String> = OpenDocs(phone.registry.clone())
        .held()
        .await
        .unwrap()
        .into_iter()
        .map(|d| (d.doc_id, d.digest))
        .collect();
    let peers = peer_client(&phone.endpoint, &laptop).await;
    let report = pull(&local, &peers, &ours, WAIT).await.unwrap();
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(report.pulled >= 1, "{report:?}");
    let pulled = phone.registry.doc(note).await.unwrap();
    assert_eq!(value(&pulled, "text").as_deref(), Some("hello"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unregistered_device_is_refused() {
    let laptop = serve(loopback().await);
    let phone = serve(loopback().await);
    let stranger = serve(loopback().await);
    let (laptop_id, phone_id) = (Uuid::new_v4(), Uuid::new_v4());
    let account = directory(&[(laptop_id, &laptop), (phone_id, &phone)]);
    laptop
        .access
        .devices
        .set_account(account_peers(&account, laptop_id));
    let song = Uuid::new_v4();
    edit(&laptop.registry.doc(song).await.unwrap(), "tempo", "120");

    // Not on the account: nothing, on either lane.
    let client = sync_client(&stranger.endpoint, laptop.addr()).await;
    assert!(
        attach(song, &client).await.is_err(),
        "an unregistered device opened a document"
    );
    let peers = peer_client(&stranger.endpoint, &laptop).await;
    assert!(peers.list().await.is_err(), "an unregistered device listed");

    // On the account: in. Then taken off it (a lost phone): out, on the
    // next call, over the connection it already had.
    let peers = peer_client(&phone.endpoint, &laptop).await;
    assert_eq!(peers.list().await.unwrap().len(), 1);
    let without_phone = DeviceDirectory {
        devices: account
            .devices
            .iter()
            .filter(|d| d.device_id != phone_id)
            .cloned()
            .collect(),
        ..account
    };
    laptop
        .access
        .devices
        .set_account(account_peers(&without_phone, laptop_id));
    assert!(
        peers.list().await.is_err(),
        "a forgotten device still listed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_on_one_network_find_each_other() {
    let lan = || async {
        iroh_link::bind_endpoint_with(iroh::SecretKey::generate(), PeerLookup::Lan)
            .await
            .expect("bind a LAN endpoint")
    };
    let laptop = serve(lan().await);
    let phone = serve(lan().await);
    iroh_link::advertise_name(&laptop.endpoint, "Cody's laptop");
    let (laptop_id, phone_id) = (Uuid::new_v4(), Uuid::new_v4());
    let account = directory(&[(laptop_id, &laptop), (phone_id, &phone)]);
    laptop
        .access
        .devices
        .set_account(account_peers(&account, laptop_id));

    // The phone knows no address and no id yet: it looks.
    let browser = LanBrowser::new(phone.endpoint.id()).expect("browse the LAN");
    let mut nearby = browser.events().await;
    let found = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match nearby.next().await {
                Some(LanEvent::Found { endpoint, name }) if endpoint == laptop.endpoint.id() => {
                    return name;
                }
                Some(_) => {}
                None => panic!("the browser stopped"),
            }
        }
    })
    .await
    .expect("the laptop is found on the network");
    assert_eq!(found.as_deref(), Some("Cody's laptop"));

    // Dialled by the bare id it found: the address comes from the LAN.
    let song = Uuid::new_v4();
    let on_laptop = laptop.registry.doc(song).await.unwrap();
    edit(&on_laptop, "tempo", "120");
    let client = sync_client(&phone.endpoint, laptop.endpoint.id()).await;
    let on_phone = CrdtDoc::ephemeral();
    let _session = live(song, on_phone.clone(), client);
    sees(
        &on_phone,
        "tempo",
        "120",
        "the phone syncs the found laptop's song",
    )
    .await;
    edit(&on_phone, "key", "D");
    sees(&on_laptop, "key", "D", "the laptop gets the phone's edit").await;
}
