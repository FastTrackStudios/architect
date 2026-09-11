//! End-to-end tests for `architect::permissions_gate` over a real vox
//! memory link: validated bearer identity (`SessionIdentityResolver`) ×
//! role engine × the router gate, exactly the org-lane wiring
//! `apps/task/server` uses.

use std::sync::Arc;

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_permissions::{
    Action, PermissionEngine, Principal, Resource, RoleEngine, Rule, ScopeEngine, StaticPrincipal,
};
use auth::AuthService as _;
use auth::identity::SessionIdentityResolver;
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
        /// Reports the caller the gate resolved, as `describe()` renders
        /// it — or `"none"` when nothing did.
        async fn who_am_i(&self) -> String;
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
        async fn who_am_i(&self) -> String {
            architect::permissions_gate::caller()
                .map_or_else(|| "none".to_string(), |who| who.describe())
        }
    }
}

use gate_probe::{
    GateProbeClient, GateProbeDispatcher, GateProbeService, gate_probe_service_descriptor,
};

const PROBE_PERMITS: architect_permissions::ServicePermits =
    architect_permissions::ServicePermits {
        service: "gate-probe",
        methods: &[
            architect_permissions::MethodPermit::new("read_thing", "read", "probe/**"),
            architect_permissions::MethodPermit::new("write_thing", "write", "probe/**"),
            architect_permissions::MethodPermit::new("who_am_i", "read", "probe/**"),
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
                std::future::pending::<()>().await;
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
        .permit(gate_probe_service_descriptor(), &PROBE_PERMITS);

    let router = LayerRouter::new().with(
        gate_probe_service_descriptor(),
        GateProbeDispatcher::new(GateProbeService),
    );
    let gated = router.with_permissions(gate);

    // Anonymous (no token): denied.
    let anon: GateProbeClient = establish_gated(gated.clone()).await;
    assert!(
        is_denied(&anon.read_thing().await),
        "anonymous read must be denied"
    );

    // Alice with her real token: read + write pass, unlisted method fails closed.
    let authed: GateProbeClient = establish_gated(gated.clone()).await;
    let authed = authed.with_middleware(AuthClientMiddleware::bearer(alice.token.clone()));
    assert_eq!(authed.read_thing().await.expect("alice reads"), "read-ok");
    assert_eq!(
        authed.write_thing().await.expect("alice writes"),
        "write-ok"
    );
    assert!(
        is_denied(&authed.secret_thing().await),
        "unlisted method must fail closed even for members"
    );

    // A garbage token resolves to Anonymous → denied.
    let forged: GateProbeClient = establish_gated(gated).await;
    let forged = forged.with_middleware(AuthClientMiddleware::bearer("not-a-real-token"));
    assert!(
        is_denied(&forged.read_thing().await),
        "forged token must be denied"
    );
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
        .permit(gate_probe_service_descriptor(), &PROBE_PERMITS);

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
    .permit(gate_probe_service_descriptor(), &PROBE_PERMITS)
    .observe_only(true);

    let router = LayerRouter::new().with(
        gate_probe_service_descriptor(),
        GateProbeDispatcher::new(GateProbeService),
    );
    let client: GateProbeClient = establish_gated(router.with_permissions(gate)).await;
    // Would be denied — observe-only audits and passes.
    assert_eq!(
        client.read_thing().await.expect("observe-only passes"),
        "read-ok"
    );
}

#[test]
fn engines_answer_direct_checks_for_handler_level_use() {
    // The in-handler fine-grained path: same engine, finer resource.
    let scope = ScopeEngine::new(vec![Rule::new("vault/Setlists/", &["read"])]);
    let g = Principal::Guest {
        link_id: "l".into(),
        display: None,
    };
    assert!(
        scope
            .check(
                &g,
                &Resource::new("vault/Setlists/Sunday Worship.md"),
                &Action::read()
            )
            .allowed()
    );
    assert!(
        !scope
            .check(&g, &Resource::new("vault/Finance/q3.md"), &Action::read())
            .allowed()
    );
}

/// The gate resolves an identity for every call it passes; a handler can
/// read it.
///
/// This is what lets a method answer differently per person. Without it a
/// caller-sensitive method has two options, and both are wrong: resolve
/// the token itself, which it cannot — the metadata borrow does not
/// outlive dispatch — or answer for a process-wide default, which is how
/// an owner shortcut becomes everyone's.
#[tokio::test]
async fn a_handler_sees_the_caller_the_gate_resolved() {
    let auth_engine = open_auth().await;
    let alice = auth_engine
        .sign_up_email_password(auth::SignUpEmailPassword {
            email: "caller@example.com".into(),
            password: "correct horse battery staple".into(),
            name: Some("Alice".into()),
            username: None,
            image: None,
            metadata_json: None,
            ip_address: None,
            user_agent: None,
        })
        .await
        .expect("sign up");

    let mut roles = RoleEngine::new();
    roles.set_member(alice.user.id.to_string(), "member");
    let gate = PermissionsGate::new(
        Arc::new(roles),
        Arc::new(SessionIdentityResolver::new(auth_engine.clone())),
    )
    .unlisted(UnlistedPolicy::Allow)
    .permit(gate_probe_service_descriptor(), &PROBE_PERMITS);
    let gated = LayerRouter::new()
        .with(
            gate_probe_service_descriptor(),
            GateProbeDispatcher::new(GateProbeService),
        )
        .with_permissions(gate);

    let signed: GateProbeClient = establish_gated(gated.clone()).await;
    let signed = signed.with_middleware(AuthClientMiddleware::bearer(alice.token.clone()));
    assert_eq!(
        signed.who_am_i().await.expect("signed-in call"),
        format!("user:{}", alice.user.id),
        "the handler saw a different principal than the gate resolved"
    );

    // The negative half. `who_am_i` is a `read` on `probe/**`, which an
    // anonymous caller cannot have — so what this proves is that the
    // identity is per-call rather than sticky from the signed-in client
    // above, which shares the router.
    let anon: GateProbeClient = establish_gated(gated).await;
    assert!(
        is_denied(&anon.who_am_i().await),
        "an anonymous caller reached a member-only method"
    );
}

// ── Coverage + the fail-closed default ──────────────────────────────────

/// A second probe service, mounted but with NO permit table — the shape
/// that used to be served silently.
mod untabled_probe {
    #[vox::service]
    pub trait UntabledProbe {
        async fn anything(&self) -> String;
    }

    #[derive(Clone)]
    pub struct UntabledProbeService;

    impl UntabledProbe for UntabledProbeService {
        async fn anything(&self) -> String {
            "served".into()
        }
    }
}

use untabled_probe::{
    UntabledProbeClient, UntabledProbeDispatcher, UntabledProbeService,
    untabled_probe_service_descriptor,
};

fn two_service_router() -> LayerRouter {
    LayerRouter::new()
        .with(
            gate_probe_service_descriptor(),
            GateProbeDispatcher::new(GateProbeService),
        )
        .with(
            untabled_probe_service_descriptor(),
            UntabledProbeDispatcher::new(UntabledProbeService),
        )
}

/// The default is fail-closed. A service nobody wrote a table for is not
/// a service anybody decided to expose.
#[tokio::test]
async fn untabled_service_is_denied_by_default() {
    let identity = SessionIdentityResolver::new(open_auth().await);
    // Note: no `.unlisted(..)` call — this is the DEFAULT.
    let gate = PermissionsGate::new(Arc::new(RoleEngine::new()), Arc::new(identity))
        .permit(gate_probe_service_descriptor(), &PROBE_PERMITS);
    let gated = two_service_router().with_permissions(gate);

    let client: UntabledProbeClient = establish_gated(gated).await;
    assert!(
        is_denied(&client.anything().await),
        "an untabled service must be denied without an explicit opt-in"
    );
}

/// …and `UnlistedPolicy::Allow` is still there for a deliberate,
/// documented migration — it just isn't what you get by accident.
#[tokio::test]
async fn untabled_service_is_served_when_explicitly_allowed() {
    let identity = SessionIdentityResolver::new(open_auth().await);
    let gate = PermissionsGate::new(Arc::new(RoleEngine::new()), Arc::new(identity))
        .unlisted(UnlistedPolicy::Allow)
        .permit(gate_probe_service_descriptor(), &PROBE_PERMITS);
    let gated = two_service_router().with_permissions(gate);

    let client: UntabledProbeClient = establish_gated(gated).await;
    assert_eq!(
        client.anything().await.expect("explicitly allowed"),
        "served"
    );
}

/// The report names every gap: the untabled service, the tabled-but-
/// unlisted method, and a permit that points at nothing.
#[tokio::test]
async fn coverage_names_every_gap() {
    const PHANTOM_PERMITS: architect_permissions::ServicePermits =
        architect_permissions::ServicePermits {
            service: "gate-probe",
            methods: &[
                architect_permissions::MethodPermit::new("read_thing", "read", "probe/**"),
                // Renamed away / never existed.
                architect_permissions::MethodPermit::new("ghost_thing", "read", "probe/**"),
            ],
        };

    let identity = SessionIdentityResolver::new(open_auth().await);
    let gate = PermissionsGate::new(Arc::new(RoleEngine::new()), Arc::new(identity))
        .permit(gate_probe_service_descriptor(), &PHANTOM_PERMITS);
    let router = two_service_router();
    let coverage = gate.coverage(&router);

    assert!(!coverage.is_complete());
    assert_eq!(coverage.services, 2, "two services mounted");
    assert_eq!(coverage.tabled, 1, "one of them has a table");
    assert_eq!(coverage.untabled, vec!["UntabledProbe"]);

    // `write_thing`, `secret_thing` and `who_am_i` are on the descriptor
    // but absent from this table → fail-closed.
    let mut uncovered: Vec<&str> = coverage.uncovered.iter().map(|(_, m)| *m).collect();
    uncovered.sort_unstable();
    assert_eq!(uncovered, vec!["secret_thing", "who_am_i", "write_thing"]);

    // …and the permit that names nothing is called out as dead.
    assert_eq!(coverage.phantom, vec![("GateProbe", "ghost_thing")]);
}

/// A table that covers the whole descriptor, on a router that mounts only
/// that service, reports complete.
#[tokio::test]
async fn complete_coverage_reports_complete() {
    const FULL_PERMITS: architect_permissions::ServicePermits =
        architect_permissions::ServicePermits {
            service: "gate-probe",
            methods: &[
                architect_permissions::MethodPermit::new("read_thing", "read", "probe/**"),
                architect_permissions::MethodPermit::new("write_thing", "write", "probe/**"),
                architect_permissions::MethodPermit::new("secret_thing", "admin", "probe/**"),
                architect_permissions::MethodPermit::new("who_am_i", "read", "probe/**"),
            ],
        };

    let identity = SessionIdentityResolver::new(open_auth().await);
    let gate = PermissionsGate::new(Arc::new(RoleEngine::new()), Arc::new(identity))
        .permit(gate_probe_service_descriptor(), &FULL_PERMITS);
    let router = LayerRouter::new().with(
        gate_probe_service_descriptor(),
        GateProbeDispatcher::new(GateProbeService),
    );
    let coverage = gate.coverage(&router);

    assert!(coverage.is_complete(), "{coverage:?}");
    assert_eq!(coverage.permitted, coverage.methods);
    assert!(coverage.untabled.is_empty());
}
