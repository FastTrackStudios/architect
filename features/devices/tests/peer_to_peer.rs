//! Devices end to end, over a real issuer (in-memory SQLite, in-process
//! vox) and real iroh endpoints on loopback (no mDNS, no relay).
//!
//! - Two devices of one account find each other through the issuer's
//!   address book (`account::pass`), a third device that is nobody's is
//!   paired by code, and all three edit one document live with the issuer
//!   stopped.
//! - Revoking a device — taking it off the account, or a pairing back —
//!   ends its live session and closes its connection; it gets nothing
//!   after.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use architect::iroh_link::{self, VOX_ALPN};
use architect::{LayerRouter, LocalServer, Scope as LocalScope};
use architect_devices::account::{self, AccountNode, FixedEndpoint, ThisDevice};
use architect_devices::{
    Admission, DevicePairingClient, PairError, PairProof, PairRequest, Pairings, Scope,
};
use architect_permissions::Principal;
use auth::transport::vox::AuthVoxService;
use auth::{AccountOrg, AuthServiceClient, DeviceServiceClient, PeerDevice, SignUpEmailPassword};
use crdt::CrdtDoc;
use crdt::p2p::{NodeAccess, Shares, node_router};
use crdt::peer::OpenDocs;
use crdt::registry::DocRegistry;
use crdt::sync::{DocSyncClient, SyncedDoc};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(10);

// ── The issuer ─────────────────────────────────────────────────────────

struct Issuer {
    server: LocalServer,
    devices: DeviceServiceClient,
}

async fn issuer() -> Issuer {
    use auth::backend_db::{AuthSeaOrmStorage, Migrator};
    use sea_orm_migration::MigratorTrait;

    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let auth = auth::ArchitectAuth::builder()
        .secret("a-secret-at-least-32-bytes-long!!")
        .storage(AuthSeaOrmStorage::new(db))
        .build()
        .unwrap();
    let router = LayerRouter::new()
        .merge(auth::auth_service_layer(AuthVoxService::new(auth.clone())))
        .merge(auth::device_service_layer(AuthVoxService::new(auth)));
    let server = LocalServer::serve(router, LocalScope::new());
    let devices = server.establish().await.unwrap();
    Issuer { server, devices }
}

async fn sign_up(issuer: &Issuer, email: &str) -> String {
    let session: AuthServiceClient = issuer.server.establish().await.unwrap();
    session
        .sign_up_email_password(SignUpEmailPassword {
            email: email.into(),
            password: "correct horse battery staple".into(),
            name: Some("Device Owner".into()),
            username: None,
            image: None,
            metadata_json: None,
            ip_address: None,
            user_agent: Some("devices-test".into()),
        })
        .await
        .unwrap()
        .token
}

// ── A node ─────────────────────────────────────────────────────────────

/// One device: its endpoint, its documents, who it lets in, its offers.
struct Node {
    endpoint: iroh::Endpoint,
    registry: DocRegistry,
    devices: Admission,
    pairings: Pairings,
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
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// A node serving its documents and its pairing offers on `endpoint`,
/// each connection closed when its device is revoked.
async fn node(name: &str) -> Node {
    let endpoint = loopback().await;
    let devices = Admission::new();
    let pairings = Pairings::new(devices.clone(), &endpoint.id().to_string(), name);
    let access = NodeAccess::new(devices.clone(), Shares::new()).pairing(pairings.clone());
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
        let devices = devices.clone();
        async move {
            architect_devices::serve(&endpoint, devices, move |remote| {
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
        devices,
        pairings,
        _serve: serve,
    }
}

/// A test node's side of the account: it offers its own org, and is at
/// its endpoint.
struct Personal {
    endpoint: String,
    org: AccountOrg,
}

impl AccountNode for Personal {
    fn offer(&self) -> Option<AccountOrg> {
        Some(self.org.clone())
    }

    async fn hold(
        &self,
        _: Option<&AccountOrg>,
        _: &[PeerDevice],
    ) -> Result<Option<String>, String> {
        Ok(Some(self.endpoint.clone()))
    }
}

fn this(name: &str) -> ThisDevice {
    ThisDevice {
        device_id: Uuid::new_v4(),
        name: name.into(),
        platform: "linux".into(),
        relay_url: None,
        retire: None,
    }
}

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap()
}

// ── Documents ──────────────────────────────────────────────────────────

async fn sync_client(from: &iroh::Endpoint, to: &Node) -> DocSyncClient {
    let link = iroh_link::connect(from, to.addr()).await.unwrap();
    vox::initiator_on(link)
        .establish::<DocSyncClient>()
        .await
        .unwrap()
}

async fn pairing_client(from: &iroh::Endpoint, to: &Node) -> DevicePairingClient {
    let link = iroh_link::connect(from, to.addr()).await.unwrap();
    vox::initiator_on(link)
        .establish::<DevicePairingClient>()
        .await
        .unwrap()
}

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

/// Pair `guest` with `host` by the code of an offer for `scopes`.
async fn pair_by_code(host: &Node, guest: &iroh::Endpoint, scopes: Vec<Scope>) {
    let offer = host.pairings.offer(scopes.clone(), now_ms()).unwrap();
    let pairing = pairing_client(guest, host).await;
    let paired = pairing
        .pair(PairRequest {
            proof: PairProof::Code { code: offer.code },
            name: "Sam's iPad".into(),
            platform: "ios".into(),
        })
        .await
        .unwrap();
    assert_eq!(paired.endpoint, host.id());
    assert_eq!(paired.scopes, scopes);
}

// ── The tests ──────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn two_account_devices_and_a_paired_one_edit_live_with_the_issuer_stopped() {
    let issuer = issuer().await;
    let token = sign_up(&issuer, "owner@example.com").await;
    let laptop = node("laptop").await;
    let phone = node("phone").await;
    let (on_laptop, on_phone) = (this("laptop"), this("phone"));
    let personal = AccountOrg {
        id: Uuid::new_v4(),
        slug: "personal".into(),
        display_name: "Personal".into(),
    };
    let laptop_node = Personal {
        endpoint: laptop.id(),
        org: personal.clone(),
    };
    let phone_node = FixedEndpoint(phone.id());

    // The laptop signs in first: its org is the account's.
    let first = account::pass(
        &issuer.devices,
        &token,
        &on_laptop,
        &laptop_node,
        &laptop.devices,
    )
    .await
    .unwrap();
    assert_eq!(first.org.as_ref(), Some(&personal));
    assert!(first.others.is_empty());
    // The phone finds the laptop; the laptop finds the phone on its next
    // pass.
    let second = account::pass(
        &issuer.devices,
        &token,
        &on_phone,
        &phone_node,
        &phone.devices,
    )
    .await
    .unwrap();
    assert_eq!(
        second.org.as_ref(),
        Some(&personal),
        "the first device's org"
    );
    assert_eq!(second.admitted, vec![laptop.id()]);
    let again = account::pass(
        &issuer.devices,
        &token,
        &on_laptop,
        &laptop_node,
        &laptop.devices,
    )
    .await
    .unwrap();
    assert_eq!(again.admitted, vec![phone.id()]);
    assert!(laptop.devices.is_account(&phone.id()));

    // The issuer stops. Nothing below reaches it.
    drop(issuer);

    // A third device, on nobody's account, is paired by code for one song.
    let ipad = loopback().await;
    let (song, setlist) = (Uuid::new_v4(), Uuid::new_v4());
    pair_by_code(&laptop, &ipad, vec![Scope::doc(song, &["read", "write"])]).await;
    let held = laptop.devices.get(&ipad.id().to_string()).unwrap();
    assert_eq!(held.name, "Sam's iPad");
    assert!(!held.is_account());

    // All three edit the song live, through the laptop.
    let at_laptop = laptop.registry.doc(song).await.unwrap();
    edit(&at_laptop, "tempo", "120");
    let at_phone = CrdtDoc::ephemeral();
    let _phone_session = live(
        song,
        at_phone.clone(),
        sync_client(&phone.endpoint, &laptop).await,
    );
    let at_ipad = CrdtDoc::ephemeral();
    let ipad_client = sync_client(&ipad, &laptop).await;
    let _ipad_session = live(song, at_ipad.clone(), ipad_client.clone());
    sees(&at_phone, "tempo", "120", "the phone gets the song").await;
    sees(&at_ipad, "tempo", "120", "the paired iPad gets the song").await;
    edit(&at_ipad, "key", "D");
    sees(&at_laptop, "key", "D", "the laptop gets the iPad's edit").await;
    sees(&at_phone, "key", "D", "the phone gets the iPad's edit").await;
    edit(&at_phone, "capo", "2");
    sees(&at_ipad, "capo", "2", "the iPad gets the phone's edit").await;

    // The pairing names the song, nothing else.
    edit(
        &laptop.registry.doc(setlist).await.unwrap(),
        "first",
        "intro",
    );
    assert!(
        attach(setlist, &ipad_client).await.is_err(),
        "a paired device opened a document its scopes do not name"
    );
    // The phone, one of the account's own, opens anything.
    let setlist_at_phone = CrdtDoc::ephemeral();
    let _setlist = live(
        setlist,
        setlist_at_phone.clone(),
        sync_client(&phone.endpoint, &laptop).await,
    );
    sees(
        &setlist_at_phone,
        "first",
        "intro",
        "an account device opens any document",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_code_pairs_once_and_a_wrong_one_nothing() {
    let laptop = node("laptop").await;
    let stranger = loopback().await;
    let offer = laptop
        .pairings
        .offer(vec![Scope::doc(Uuid::new_v4(), &["read"])], now_ms())
        .unwrap();
    let wrong = if offer.code == "000000" {
        "000001"
    } else {
        "000000"
    };
    let pairing = pairing_client(&stranger, &laptop).await;
    let request = |code: &str| PairRequest {
        proof: PairProof::Code { code: code.into() },
        name: "stranger".into(),
        platform: "linux".into(),
    };
    let refused = pairing.pair(request(wrong)).await;
    assert!(
        matches!(&refused, Err(vox::VoxError::User(e)) if **e == PairError::NoOffer),
        "{refused:?}"
    );
    assert!(!laptop.devices.admits(&stranger.id().to_string()));
    // A stranger may ask to pair, and nothing else.
    assert!(
        attach(Uuid::new_v4(), &sync_client(&stranger, &laptop).await)
            .await
            .is_err()
    );

    pairing.pair(request(&offer.code)).await.unwrap();
    assert!(laptop.devices.admits(&stranger.id().to_string()));
    let other = loopback().await;
    let again = pairing_client(&other, &laptop)
        .await
        .pair(request(&offer.code))
        .await;
    assert!(again.is_err(), "an offer is used once");
    assert!(!laptop.devices.admits(&other.id().to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_closes_its_lanes() {
    let issuer = issuer().await;
    let token = sign_up(&issuer, "owner@example.com").await;
    let laptop = node("laptop").await;
    let phone = node("phone").await;
    let (on_laptop, on_phone) = (this("laptop"), this("phone"));
    let (laptop_node, phone_node) = (FixedEndpoint(laptop.id()), FixedEndpoint(phone.id()));
    for _ in 0..2 {
        account::pass(
            &issuer.devices,
            &token,
            &on_laptop,
            &laptop_node,
            &laptop.devices,
        )
        .await
        .unwrap();
        account::pass(
            &issuer.devices,
            &token,
            &on_phone,
            &phone_node,
            &phone.devices,
        )
        .await
        .unwrap();
    }
    let ipad = loopback().await;
    let song = Uuid::new_v4();
    pair_by_code(&laptop, &ipad, vec![Scope::doc(song, &["write"])]).await;

    let at_laptop = laptop.registry.doc(song).await.unwrap();
    edit(&at_laptop, "tempo", "120");
    let at_phone = CrdtDoc::ephemeral();
    let phone_client = sync_client(&phone.endpoint, &laptop).await;
    let phone_session = live(song, at_phone.clone(), phone_client.clone());
    let at_ipad = CrdtDoc::ephemeral();
    let ipad_client = sync_client(&ipad, &laptop).await;
    let ipad_session = live(song, at_ipad.clone(), ipad_client.clone());
    sees(&at_phone, "tempo", "120", "the phone joins").await;
    sees(&at_ipad, "tempo", "120", "the iPad joins").await;

    // The pairing is taken back: the iPad's session ends, its connection
    // closes, and a new one gets nothing.
    assert!(laptop.devices.revoke(&ipad.id().to_string()).is_some());
    tokio::time::timeout(WAIT, ipad_session)
        .await
        .expect("a revoked device's session ended")
        .unwrap();
    assert!(
        attach(song, &ipad_client).await.is_err(),
        "the revoked device's old connection still opens the song"
    );
    assert!(
        attach(song, &sync_client(&ipad, &laptop).await)
            .await
            .is_err(),
        "a revoked device dialled back in"
    );

    // The phone is taken off the account: the laptop's next pass lets it
    // in no more, and its session there ends too.
    issuer
        .devices
        .forget_device(token.clone(), on_phone.device_id)
        .await
        .unwrap();
    let after = account::pass(
        &issuer.devices,
        &token,
        &on_laptop,
        &laptop_node,
        &laptop.devices,
    )
    .await
    .unwrap();
    assert!(after.admitted.is_empty());
    tokio::time::timeout(WAIT, phone_session)
        .await
        .expect("a device taken off the account lost its session")
        .unwrap();
    assert!(
        attach(song, &phone_client).await.is_err(),
        "a forgotten device still opens the song"
    );
}
