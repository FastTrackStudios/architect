//! An org's lane on an `EngineHost` (`vox_at("/org/{slug}/vox", …)`):
//! the org is entered around the host's gate, so `ConfinedEngine` sees it
//! when the gate checks the call, and a member of one org calling
//! another org's lane is refused there, before any handler runs.
//!
//! The server and client are in-process: a `WebSocket` over
//! `127.0.0.1:0`.

#![cfg(feature = "server")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::net::SocketAddr;
use std::sync::Arc;

use architect::connect::{Endpoint, Pool};
use architect::host::{EngineHost, Lane, RequestHead};
use architect::http::axum::http::StatusCode;
use architect::layer::LayerRouter;
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_orgs::server::{BoxOrgsFuture, ConfinedEngine, Directory, Members, MembersResolver};
use architect_orgs::{Org, OrgId, Role};
use architect_permissions::{
    BoxIdentityFuture, IdentityResolver, MethodPermit, Principal, Rule, ServicePermits,
};

mod probe {
    #[vox::service]
    pub trait LaneProbe {
        async fn read_note(&self) -> String;
        /// The org the call ran in, or `"none"`.
        async fn where_am_i(&self) -> String;
    }

    #[derive(Clone)]
    pub struct LaneProbeService;

    impl LaneProbe for LaneProbeService {
        async fn read_note(&self) -> String {
            "read".into()
        }
        async fn where_am_i(&self) -> String {
            architect_orgs::server::current_org().map_or_else(|| "none".into(), |o| o.0)
        }
    }
}

use probe::{
    LaneProbeClient, LaneProbeDispatcher, LaneProbeService, lane_probe_service_descriptor,
};

const PERMITS: ServicePermits = ServicePermits {
    service: "lane-probe",
    methods: &[
        MethodPermit::new("read_note", "read", "notes/**"),
        MethodPermit::new("where_am_i", "read", "notes/**"),
    ],
};

/// Bob is a member of `hp` only; alice of `fts` only.
struct Identity;

impl Directory for Identity {
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a> {
        Box::pin(async move {
            match token {
                "alice" => Ok(vec![Org::new("fts", "FastTrackStudio", Role::Member)]),
                "bob" => Ok(vec![Org::new("hp", "Harbor & Pine", Role::Member)]),
                _ => Err("unknown session".into()),
            }
        })
    }
}

/// Sessions: the token is the user id.
struct Sessions;

impl IdentityResolver for Sessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = bearer_token.map_or(Principal::Anonymous, |t| Principal::User {
            user_id: t.to_owned(),
        });
        Box::pin(async move { who })
    }
}

fn router() -> LayerRouter {
    LayerRouter::new().with(
        lane_probe_service_descriptor(),
        LaneProbeDispatcher::new(LaneProbeService),
    )
}

/// The gate: `ConfinedEngine` (members may read and write), over
/// sessions whose orgs the identity server answers.
fn gate(engine: impl FnOnce(ConfinedEngine) -> ConfinedEngine) -> PermissionsGate {
    let members = Members::new();
    let engine = engine(
        ConfinedEngine::new(members.clone())
            .role(&Role::Member, vec![Rule::new("**", &["read", "write"])]),
    );
    let identity = MembersResolver::new(Sessions, Identity, members);
    PermissionsGate::new(Arc::new(engine), Arc::new(identity))
        .unlisted(UnlistedPolicy::Deny)
        .permit(lane_probe_service_descriptor(), &PERMITS)
}

/// A host serving `fts` and `hp` each on its own `/org/{slug}/vox` lane,
/// the way the docs build it.
fn host() -> EngineHost {
    EngineHost::new(router(), gate(|e| e), "127.0.0.1:0").vox_at(
        "/org/{slug}/vox",
        |head: RequestHead| async move {
            match head.param("slug") {
                Some(slug @ ("fts" | "hp")) => Ok(Lane::new(router()).scoped(OrgId::new(slug))),
                _ => Err(StatusCode::NOT_FOUND),
            }
        },
    )
}

async fn spawn(host: EngineHost) -> SocketAddr {
    let app = host.into_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await });
    at
}

/// A client on `org`'s lane, signed in as `who`. A pool of its own: the
/// identity is the connection's.
async fn connect(at: SocketAddr, org: &str, who: &str) -> LaneProbeClient {
    let endpoint =
        Endpoint::url(format!("ws://{at}/org/{org}/vox")).with_bearer(Some(who.to_owned()));
    Pool::default().client(&endpoint).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_of_one_org_is_refused_at_the_gate_on_anothers_lane() {
    let at = spawn(host()).await;

    let bob_hp = connect(at, "hp", "bob").await;
    assert_eq!(bob_hp.where_am_i().await.unwrap(), "hp");
    assert_eq!(bob_hp.read_note().await.unwrap(), "read");

    let bob_fts = connect(at, "fts", "bob").await;
    let refused = bob_fts.read_note().await.unwrap_err().to_string();
    assert!(refused.contains("permission denied"), "{refused}");
    assert!(refused.contains("not a member of org fts"), "{refused}");
    assert!(bob_fts.where_am_i().await.is_err());

    let alice_fts = connect(at, "fts", "alice").await;
    assert_eq!(alice_fts.where_am_i().await.unwrap(), "fts");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_org_lane_left_unscoped_fails_closed_when_the_engine_requires_an_org() {
    // The resolver forgot the scope: a bare router on an org's path. An
    // engine for a host with no All-organizations lane refuses every call
    // there instead of reading it as one.
    let host = EngineHost::new(router(), gate(ConfinedEngine::require_org), "127.0.0.1:0")
        .vox_at("/org/{slug}/vox", |_: RequestHead| async move {
            Ok::<_, StatusCode>(router())
        });
    let at = spawn(host).await;

    let bob = connect(at, "hp", "bob").await;
    let refused = bob.read_note().await.unwrap_err().to_string();
    assert!(refused.contains("permission denied"), "{refused}");
    assert!(refused.contains("in no org"), "{refused}");
}
