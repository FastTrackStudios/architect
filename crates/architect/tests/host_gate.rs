//! `EngineHost` gates every vox lane it owns: the built-in `/vox`, a
//! `vox_at` lane and the iroh lane each refuse a caller the gate does not
//! permit and serve one it does. A lane's scope is entered outside the
//! gate, so the engine checks each call in it. On iroh the caller is the endpoint id
//! the handshake proved. `EngineHost::ungated` is the explicit opt-out.
//!
//! Every server and client is in-process on loopback: `WebSocket`s over
//! `127.0.0.1:0`, and iroh endpoints with relays off dialled by direct
//! address (no n0, no mDNS), so this runs in CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::net::SocketAddr;
use std::sync::Arc;

use architect::LayerRouter;
use architect::connect::{Endpoint, Pool};
use architect::host::{EngineHost, Lane, RequestHead};
use architect::http::axum::http::StatusCode;
use architect::permissions_gate::{LaneScope, PermissionsGate, StaticPrincipal};
use architect_permissions::{
    Action, BoxIdentityFuture, Decision, IdentityResolver, MethodPermit, PermissionEngine,
    Principal, Resource, ServicePermits,
};

mod probe {
    /// Answers with the caller the gate resolved.
    #[vox::service]
    pub trait GateProbe {
        /// `describe()` of the caller, or `"none"` when nothing resolved one.
        async fn who_am_i(&self) -> String;
    }

    #[derive(Clone)]
    pub struct GateProbeService;

    impl GateProbe for GateProbeService {
        async fn who_am_i(&self) -> String {
            architect::permissions_gate::caller()
                .map_or_else(|| "none".to_owned(), |who| who.describe())
        }
    }
}

use probe::{
    GateProbeClient, GateProbeDispatcher, GateProbeService, gate_probe_service_descriptor,
};

const PERMITS: ServicePermits = ServicePermits {
    service: "gate-probe",
    methods: &[MethodPermit::new("who_am_i", "read", "probe/**")],
};

/// The bearer is the user id.
struct Sessions;

impl IdentityResolver for Sessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = bearer_token.map_or(Principal::Anonymous, |t| Principal::User {
            user_id: t.to_owned(),
        });
        Box::pin(async move { who })
    }
}

/// Lets in the user `alice` and the one peer endpoint it names; nobody
/// else.
struct Admits {
    peer: Option<String>,
}

impl PermissionEngine for Admits {
    fn check(&self, who: &Principal, _what: &Resource, _action: &Action) -> Decision {
        let admitted = match who {
            Principal::User { user_id } => user_id == "alice",
            Principal::Host { endpoint } => self.peer.as_deref() == Some(endpoint.as_str()),
            _ => false,
        };
        if admitted {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: format!("{} is not admitted", who.describe()),
            }
        }
    }
}

fn router() -> LayerRouter {
    LayerRouter::new().with(
        gate_probe_service_descriptor(),
        GateProbeDispatcher::new(GateProbeService),
    )
}

fn gate(peer: Option<String>) -> PermissionsGate {
    PermissionsGate::new(Arc::new(Admits { peer }), Arc::new(Sessions))
        .permit(gate_probe_service_descriptor(), &PERMITS)
}

async fn spawn(host: EngineHost) -> SocketAddr {
    let app = host.into_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await });
    at
}

/// `who_am_i` over a fresh WebSocket to `path`, as `bearer`.
async fn ask(at: SocketAddr, path: &str, bearer: Option<&str>) -> Result<String, String> {
    // A pool of its own per call: the process-global pool would hand a
    // later call an earlier call's connection, and identity is per
    // connection.
    let endpoint = Endpoint::url(format!("ws://{at}{path}")).with_bearer(bearer.map(str::to_owned));
    let client: GateProbeClient = Pool::default()
        .client(&endpoint)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    client.who_am_i().await.map_err(|e| e.to_string())
}

fn assert_denied(result: Result<String, String>, lane: &str) {
    let err = result.expect_err(&format!("{lane}: an unpermitted caller was served"));
    assert!(err.contains("permission denied"), "{lane}: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_built_in_vox_is_gated_by_the_upgrade_bearer() {
    let at = spawn(EngineHost::new(router(), gate(None), "127.0.0.1:0")).await;

    assert_denied(ask(at, "/vox", None).await, "/vox anonymous");
    assert_denied(ask(at, "/vox", Some("mallory")).await, "/vox mallory");
    assert_eq!(ask(at, "/vox", Some("alice")).await.unwrap(), "user:alice");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vox_at_lane_is_gated_by_the_host() {
    let host = EngineHost::new(router(), gate(None), "127.0.0.1:0").vox_at(
        "/org/{slug}/vox",
        |head: RequestHead| async move {
            match head.param("slug") {
                // A bare router: the host puts its gate in front of it.
                Some("acme") => Ok(router()),
                _ => Err(StatusCode::NOT_FOUND),
            }
        },
    );
    let at = spawn(host).await;

    assert_denied(ask(at, "/org/acme/vox", None).await, "vox_at anonymous");
    assert_denied(
        ask(at, "/org/acme/vox", Some("mallory")).await,
        "vox_at mallory",
    );
    assert_eq!(
        ask(at, "/org/acme/vox", Some("alice")).await.unwrap(),
        "user:alice"
    );
}

tokio::task_local! {
    /// The tenant a call runs in: a lane scope the gate's engine reads.
    static TENANT: String;
}

/// A lane's tenant, entered around the gate.
#[derive(Clone)]
struct Tenant(String);

impl LaneScope for Tenant {
    fn enter<F>(&self, call: F) -> impl Future<Output = F::Output> + Send
    where
        F: Future + Send,
    {
        TENANT.scope(self.0.clone(), call)
    }
}

/// Lets alice into `acme` only: what it decides depends on the scope
/// being set while the gate checks.
struct AcmeOnly;

impl PermissionEngine for AcmeOnly {
    fn check(&self, who: &Principal, _what: &Resource, _action: &Action) -> Decision {
        let tenant = TENANT.try_with(Clone::clone).ok();
        match (who, tenant.as_deref()) {
            (Principal::User { user_id }, Some("acme")) if user_id == "alice" => Decision::Allow,
            _ => Decision::Deny {
                reason: format!("{} not in {tenant:?}", who.describe()),
            },
        }
    }
}

fn tenant_gate() -> PermissionsGate {
    PermissionsGate::new(Arc::new(AcmeOnly), Arc::new(Sessions))
        .permit(gate_probe_service_descriptor(), &PERMITS)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vox_at_lanes_scope_is_entered_outside_the_hosts_gate() {
    let host = EngineHost::new(router(), tenant_gate(), "127.0.0.1:0").vox_at(
        "/t/{tenant}/vox",
        |head: RequestHead| async move {
            let tenant = Tenant(head.param("tenant").unwrap_or_default().to_owned());
            Ok::<_, StatusCode>(Lane::new(router()).scoped(tenant))
        },
    );
    let at = spawn(host).await;

    assert_eq!(
        ask(at, "/t/acme/vox", Some("alice")).await.unwrap(),
        "user:alice",
        "the gate saw the lane's tenant"
    );
    assert_denied(
        ask(at, "/t/other/vox", Some("alice")).await,
        "another tenant",
    );
    // `/vox` is in no scope: the engine sees no tenant.
    assert_denied(ask(at, "/vox", Some("alice")).await, "/vox, no tenant");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permissioned_lane_may_be_scoped_outside_its_gate() {
    let lane_gate = Arc::new(tenant_gate());
    let host = EngineHost::new(router(), gate(None), "127.0.0.1:0").vox_at_permissioned(
        "/t/{tenant}/vox",
        move |head: RequestHead| {
            let lane_gate = Arc::clone(&lane_gate);
            async move {
                let tenant = Tenant(head.param("tenant").unwrap_or_default().to_owned());
                let bearer = head.bearer().map(str::to_owned);
                Ok::<_, StatusCode>(
                    PermissionsGate::wrap_shared_with_bearer(lane_gate, router(), bearer)
                        .scoped(tenant),
                )
            }
        },
    );
    let at = spawn(host).await;

    assert_eq!(
        ask(at, "/t/acme/vox", Some("alice")).await.unwrap(),
        "user:alice"
    );
    assert_denied(
        ask(at, "/t/other/vox", Some("alice")).await,
        "another tenant",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permissioned_lane_keeps_its_own_gate() {
    // A share lane: every caller is the link's guest, whatever they
    // present, and the lane's own engine admits guests. The host's gate
    // (which would refuse a guest) is not stacked on top.
    let host = EngineHost::new(router(), gate(None), "127.0.0.1:0").vox_at_permissioned(
        "/share/{link}/vox",
        |head: RequestHead| async move {
            let guest = Principal::Guest {
                link_id: head.param("link").unwrap_or_default().to_owned(),
                display: None,
            };
            let lane = PermissionsGate::new(
                Arc::new(architect_permissions::AllowAll),
                Arc::new(StaticPrincipal(guest)),
            )
            .permit(gate_probe_service_descriptor(), &PERMITS);
            Ok::<_, StatusCode>(PermissionsGate::wrap_shared(Arc::new(lane), router()))
        },
    );
    let at = spawn(host).await;

    assert_eq!(
        ask(at, "/share/l1/vox", None).await.unwrap(),
        "guest:l1",
        "the lane's own gate resolved the caller"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ungated_host_serves_everyone_with_no_caller() {
    let at = spawn(EngineHost::ungated(router(), "127.0.0.1:0")).await;

    assert_eq!(ask(at, "/vox", None).await.unwrap(), "none");
}

#[cfg(feature = "iroh")]
mod iroh_lane {
    use super::*;
    use architect::iroh_link::{self, VOX_ALPN};

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

    async fn ask_over_iroh(
        from: &iroh::Endpoint,
        to: iroh::EndpointAddr,
    ) -> Result<String, String> {
        let link = iroh_link::connect(from, to).await.expect("dial");
        let client = vox::initiator_on(link)
            .establish::<GateProbeClient>()
            .await
            .expect("vox handshake over iroh");
        client.who_am_i().await.map_err(|e| e.to_string())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_iroh_lane_is_gated_by_the_proved_endpoint() {
        let server = loopback().await;
        let device = loopback().await;
        let stranger = loopback().await;
        let device_id = device.id().to_string();

        let host = EngineHost::new(router(), gate(Some(device_id.clone())), "127.0.0.1:0")
            .iroh_endpoint(server.clone());
        let serving = tokio::spawn(host.serve());
        let at = direct_addr(&server);

        assert_denied(ask_over_iroh(&stranger, at.clone()).await, "iroh stranger");
        assert_eq!(
            ask_over_iroh(&device, at).await.unwrap(),
            format!("host:{device_id}"),
            "the caller is the endpoint the handshake proved"
        );
        serving.abort();
    }
}
