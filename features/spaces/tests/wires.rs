//! The same space over the other wires: a person from another org, a
//! commenter in a space, reads and comments on its document and nothing
//! else over a WebSocket (an `EngineHost` org lane, the token on the
//! upgrade) and over iroh (the token on each call, the device proved by
//! the handshake); removing them binds on their next call on the same
//! connection. On the host's lane the org is entered outside the gate,
//! so a person in no space of it is refused at the gate.
//!
//! Every server and client is in-process on loopback: the WebSocket over
//! `127.0.0.1:0`, iroh endpoints with relays off dialled by direct
//! address, so this runs in CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use architect::connect::{Endpoint, Pool};
use architect::host::{EngineHost, Lane, RequestHead};
use architect::http::axum::http::StatusCode;
use architect::http::axum::serve;
use architect::iroh_link::{self, VOX_ALPN};
use architect::local::RawLaneCaller;
use architect::permissions_gate::{AUTHORIZATION_METADATA_KEY, PermissionsGate};
use architect_orgs::OrgId;
use architect_orgs::server::confine;
use architect_permissions::Principal;
use architect_spaces::server::MemoryStore;
use architect_spaces::{COMMENTER, NewSpace, SpaceId, SpacesService, SpacesServiceClient, Subject};
use common::{WorkspaceClient, World, hp, sign_in, user, world};
use uuid::Uuid;

const DOC: Uuid = Uuid::from_u128(42);
const OTHER: Uuid = Uuid::from_u128(43);

/// Alice's space over one document.
async fn a_space(world: &World) -> SpaceId {
    sign_in(world, "alice").await;
    world
        .spaces
        .acting(hp(), user("alice"))
        .create_space(NewSpace {
            name: "Review".into(),
            kind: "document".into(),
            scope: vec![format!("doc/{DOC}")],
            exclude: Vec::new(),
        })
        .await
        .unwrap()
        .id
}

/// Carol's view of the space over `docs`, from before she is added to
/// after she is removed.
async fn carol_comments_then_is_removed(world: &World, docs: &WorkspaceClient, wire: &str) {
    let space = a_space(world).await;
    let carol = Subject::User("carol".into());
    let alice = world.spaces.acting(hp(), user("alice"));

    assert!(
        docs.read_doc(DOC.to_string()).await.is_err(),
        "{wire}: an outsider read before she was added"
    );

    alice
        .set_member(space.clone(), carol.clone(), Some(COMMENTER.into()), None)
        .await
        .unwrap();
    assert_eq!(
        docs.read_doc(DOC.to_string()).await.unwrap(),
        format!("contents of {DOC}"),
        "{wire}"
    );
    assert_eq!(
        docs.comment(DOC.to_string(), "ship it".into())
            .await
            .unwrap(),
        format!("user:carol on {DOC}: ship it"),
        "{wire}"
    );
    let refused = docs
        .write_doc(DOC.to_string(), "x".into())
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("permission denied"), "{wire}: {refused}");
    assert!(
        docs.read_doc(OTHER.to_string()).await.is_err(),
        "{wire}: a document outside the space"
    );

    alice.remove_member(space, carol).await.unwrap();
    assert!(
        docs.read_doc(DOC.to_string()).await.is_err(),
        "{wire}: removal did not bind on the next call"
    );
    assert!(docs.comment(DOC.to_string(), "?".into()).await.is_err());
}

/// `/org/{slug}/vox` on an `EngineHost`, built the documented way: the
/// resolver returns the slug's org as the lane's scope, and the host
/// gates each connection with its upgrade's bearer inside that scope.
async fn serve_ws(world: &World) -> SocketAddr {
    let router = world.router.clone();
    let host = EngineHost::new(router.clone(), (*world.gate).clone(), "127.0.0.1:0").vox_at(
        "/org/{slug}/vox",
        move |head: RequestHead| {
            let router = router.clone();
            async move {
                let org = OrgId::new(head.param("slug").ok_or(StatusCode::NOT_FOUND)?);
                Ok::<_, StatusCode>(Lane::new(router).scoped(org))
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    let app = host.into_app();
    tokio::spawn(async move { serve(listener, app).await });
    at
}

/// `who`'s workspace and spaces on `hp`'s lane over a WebSocket.
async fn on_hp(at: SocketAddr, who: &str) -> (WorkspaceClient, SpacesServiceClient) {
    let endpoint = Endpoint::url(format!("ws://{at}/org/hp/vox")).with_bearer(Some(who.to_owned()));
    let pool = Pool::default();
    (
        pool.client(&endpoint).await.unwrap(),
        pool.client(&endpoint).await.unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn over_a_websocket() {
    let world = world(MemoryStore::new()).await;
    let at = serve_ws(&world).await;
    let (docs, _) = on_hp(at, "carol").await;
    carol_comments_then_is_removed(&world, &docs, "websocket").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_from_another_org_is_refused_at_the_gate() {
    let world = world(MemoryStore::new()).await;
    a_space(&world).await;
    let at = serve_ws(&world).await;

    // Carol is in `fts` and in no space of `hp`: the gate sees `hp` and
    // refuses her, before any handler runs.
    let (docs, spaces) = on_hp(at, "carol").await;
    let refused = docs
        .read_doc(DOC.to_string())
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("permission denied"), "{refused}");
    let refused = spaces.spaces().await.unwrap_err().to_string();
    assert!(refused.contains("permission denied"), "{refused}");

    // Alice owns `hp`: the same lane serves her.
    let (docs, spaces) = on_hp(at, "alice").await;
    docs.read_doc(DOC.to_string()).await.unwrap();
    assert_eq!(spaces.spaces().await.unwrap().len(), 1);
}

/// Presents `Bearer <token>` on every call: a person signed in on a peer
/// device.
struct Bearer(&'static str);

impl vox::ClientMiddleware for Bearer {
    fn pre<'a>(
        &'a self,
        _context: &'a vox::ClientContext<'a>,
        request: &'a mut vox::ClientRequest<'_, 'a>,
    ) -> vox::BoxMiddlewareFuture<'a> {
        Box::pin(async move {
            request.push_string_metadata(AUTHORIZATION_METADATA_KEY, format!("Bearer {}", self.0));
        })
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

fn direct_addr(endpoint: &iroh::Endpoint) -> iroh::EndpointAddr {
    iroh::EndpointAddr::from_parts(
        endpoint.id(),
        endpoint
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn over_iroh() {
    let world = world(MemoryStore::new()).await;
    let server = loopback().await;
    let device = loopback().await;
    let (gate, router) = (Arc::clone(&world.gate), world.router.clone());
    let serving = tokio::spawn({
        let server = server.clone();
        async move {
            iroh_link::serve_peers(&server, move |remote| {
                confine(
                    hp(),
                    PermissionsGate::wrap_shared_with_proved_caller(
                        Arc::clone(&gate),
                        router.clone(),
                        Principal::Host {
                            endpoint: remote.to_string(),
                        },
                    ),
                )
            })
            .await;
        }
    });

    let link = iroh_link::connect(&device, direct_addr(&server))
        .await
        .expect("dial");
    let lane = vox::initiator_on(link)
        .establish::<RawLaneCaller>()
        .await
        .expect("vox handshake over iroh");
    let caller = lane.caller.clone();
    let docs = WorkspaceClient::new(caller).with_middleware(Bearer("carol"));

    carol_comments_then_is_removed(&world, &docs, "iroh").await;

    // Carol back in: the same device with no person signed in on it is
    // still no member — the space holds people, not machines.
    let space = a_space(&world).await;
    world
        .spaces
        .acting(hp(), user("alice"))
        .set_member(
            space,
            Subject::User("carol".into()),
            Some(COMMENTER.into()),
            None,
        )
        .await
        .unwrap();
    docs.read_doc(DOC.to_string()).await.unwrap();
    let bare = WorkspaceClient::new(lane.caller.clone());
    assert!(bare.read_doc(DOC.to_string()).await.is_err());
    drop(lane);
    serving.abort();
}
