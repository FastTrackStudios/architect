//! End-to-end tests for `architect::permissions_gate` over a real vox
//! memory link: validated bearer identity (`SessionIdentityResolver`) ×
//! role engine × the router gate, exactly the org-lane wiring
//! `apps/task/server` uses.

use std::sync::Arc;

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_permissions::{Action, PermissionEngine, Principal, Resource, RoleEngine, Rule, ScopeEngine, StaticPrincipal};
use auth::identity::SessionIdentityResolver;
use auth::AuthService as _;
use auth::transport::vox::AuthClientMiddleware;

mod gate_probe {
    /// Two-verb probe: the gate must let `read_thing` through for readers
    /// and stop `write_thing` for non-writers — and fail closed on methods
    /// missing from the permit table.
    #[vox::service]
    pub trait GateProbe {
        async fn read_thing(&self) -> String;
        async fn write_thing(&self) -> String;
        /// Deliberately NOT in the permit table — must be denied.
        async fn secret_thing(&self) -> String;
    }

    #[derive(Clone)]
    pub struct GateProbeService;

    impl GateProbe for GateProbeService {
        async fn read_thing(&self) -> String {
            "read-ok".into()
        }
        async fn write_thing(&self) -> String {
            "write-ok".into()
        }
        async fn secret_thing(&self) -> String {
            "secret".into()
        }
    }
}

use gate_probe::{GateProbeClient, GateProbeDispatcher, GateProbeService, gate_probe_service_descriptor};

const PROBE_PERMITS: architect_permissions::ServicePermits = architect_permissions::ServicePermits {
    service: "gate-probe",
    methods: &[
        architect_permissions::MethodPermit::new("read_thing", "read", "probe/**"),
        architect_permissions::MethodPermit::new("write_thing", "write", "probe/**"),
        // secret_thing intentionally unlisted → fail-closed deny.
    ],
};

async fn open_auth() -> auth::ArchitectAuth<auth::backend_db::AuthSeaOrmStorage> {
    use auth::backend_db::{AuthSeaOrmStorage, Migrator};
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;
    let db = Database::connect("sqlite::memory:").await.expect("connect");
    Migrator::up(&db, None).await.expect("migrate");
    auth::ArchitectAuth::builder()
        .secret("a-secret-at-least-32-bytes-long!!")
        .storage(AuthSeaOrmStorage::new(db))
        .build()
        .expect("build auth")
}

/// Establish a typed client against a PERMISSIONED router over a memory
/// link (mirrors `LocalServer::establish`, which only takes bare routers).
async fn establish_gated<C>(gated: architect::permissions_gate::PermissionedRouter) -> C
where
    C: vox::FromVoxLane,
{
    let (client_link, server_link) = vox::memory_link_pair(16);
    tokio::spawn(async move {
        match vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(gated))
            .establish_connection()
            .await
        {
            Ok(connection) => {
                let _hold = connection;
                std::future::pending::<()>().await
            }
            Err(e) => panic!("acceptor: {e:?}"),
        }
    });
    vox::initiator_on(client_link)
        .establish::<C>()
        .await
        .expect("establish client")
}

/// A denied call FAILS. The reason string only survives to the client once
/// the method's response schema is established (first-call denies get
/// schema-mangled into a bare `InvalidPayload` — see the gate module docs),
/// so tests assert on failure, not on the exact message.
fn is_denied<T: std::fmt::Debug, E: std::fmt::Debug>(r: &Result<T, E>) -> bool {
    r.is_err()
}

#[tokio::test]
async fn gate_enforces_roles_over_validated_sessions() {
    let auth_engine = open_auth().await;

    // A real signed-up user with a real session token.
    let alice = auth_engine
        .sign_up_email_password(auth::SignUpEmailPassword {
            email: "alice@example.com".into(),
            password: "correct horse battery staple".into(),
            name: Some("Alice".into()),
            username: None,
            image: None,
            metadata_json: None,
            ip_address: None,
            user_agent: None,
        })
        .await
        .expect("sign up alice");

    // Alice is a member (read/write, no admin); nobody else is anything.
    let mut roles = RoleEngine::new();
    roles.set_member(alice.user.id.to_string(), "member");

    let identity = SessionIdentityResolver::new(auth_engine.clone());
    let gate = PermissionsGate::new(Arc::new(roles), Arc::new(identity))
        .unlisted(UnlistedPolicy::Allow)
        .permit(gate_probe_service_descriptor(), PROBE_PERMITS);

    let router = LayerRouter::new().with(
        gate_probe_service_descriptor(),
        GateProbeDispatcher::new(GateProbeService),
    );
    let gated = router.with_permissions(gate);

    // Anonymous (no token): denied.
    let anon: GateProbeClient = establish_gated(gated.clone()).await;
    assert!(is_denied(&anon.read_thing().await), "anonymous read must be denied");

    // Alice with her real token: read + write pass, unlisted method fails closed.
    let authed: GateProbeClient = establish_gated(gated.clone()).await;
    let authed = authed.with_middleware(AuthClientMiddleware::bearer(alice.token.clone()));
    assert_eq!(authed.read_thing().await.expect("alice reads"), "read-ok");
    assert_eq!(authed.write_thing().await.expect("alice writes"), "write-ok");
    assert!(
        is_denied(&authed.secret_thing().await),
        "unlisted method must fail closed even for members"
    );

    // A garbage token resolves to Anonymous → denied.
    let forged: GateProbeClient = establish_gated(gated).await;
    let forged = forged.with_middleware(AuthClientMiddleware::bearer("not-a-real-token"));
    assert!(is_denied(&forged.read_thing().await), "forged token must be denied");
}

#[tokio::test]
async fn share_lane_scope_engine_gates_by_prefix() {
    // The share-lane wiring: fixed Guest principal + a materialized scope.
    let scope = ScopeEngine::new(vec![Rule::new("probe/", &["read"])]);
    let guest = StaticPrincipal(Principal::Guest {
        link_id: "link-1".into(),
        display: Some("Band".into()),
    });
    let gate = PermissionsGate::new(Arc::new(scope), Arc::new(guest))
        .unlisted(UnlistedPolicy::Deny)
        .permit(gate_probe_service_descriptor(), PROBE_PERMITS);

    let router = LayerRouter::new().with(
        gate_probe_service_descriptor(),
        GateProbeDispatcher::new(GateProbeService),
    );
    let client: GateProbeClient = establish_gated(router.with_permissions(gate)).await;

    assert_eq!(client.read_thing().await.expect("guest reads"), "read-ok");
    assert!(
        is_denied(&client.write_thing().await),
        "view-only scope must deny writes"
    );
    // The read schema is established now — a SECOND denied write carries
    // the reason verbatim.
    let again = client.write_thing().await;
    if let Err(e) = &again {
        assert!(
            format!("{e:?}").contains("permission denied"),
            "established-schema deny should carry the reason: {e:?}"
        );
    }
    assert!(is_denied(&client.secret_thing().await));
}

#[tokio::test]
async fn observe_only_lets_denies_through() {
    let scope = ScopeEngine::new(vec![]); // denies everything
    let gate = PermissionsGate::new(
        Arc::new(scope),
        Arc::new(StaticPrincipal(Principal::Anonymous)),
    )
    .permit(gate_probe_service_descriptor(), PROBE_PERMITS)
    .observe_only(true);

    let router = LayerRouter::new().with(
        gate_probe_service_descriptor(),
        GateProbeDispatcher::new(GateProbeService),
    );
    let client: GateProbeClient = establish_gated(router.with_permissions(gate)).await;
    // Would be denied — observe-only audits and passes.
    assert_eq!(client.read_thing().await.expect("observe-only passes"), "read-ok");
}

#[test]
fn engines_answer_direct_checks_for_handler_level_use() {
    // The in-handler fine-grained path: same engine, finer resource.
    let scope = ScopeEngine::new(vec![Rule::new("vault/Setlists/", &["read"])]);
    let g = Principal::Guest { link_id: "l".into(), display: None };
    assert!(scope
        .check(&g, &Resource::new("vault/Setlists/Sunday Worship.md"), &Action::read())
        .allowed());
    assert!(!scope
        .check(&g, &Resource::new("vault/Finance/q3.md"), &Action::read())
        .allowed());
}
