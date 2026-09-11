//! One suite, every transport.
//!
//! The auth services are declared once (`auth_proto::AuthService`,
//! `auth_proto::OrganizationService`) and the framework generates every
//! face of them. This file is the proof that the faces agree: each
//! scenario below is written once against a small [`Surface`] adapter
//! and then run over
//!
//! * **HTTP+JSON** — the generated axum router on a real socket, driven
//!   by the generated `AuthServiceHttpClient` / `OrganizationServiceHttpClient`;
//!
//! Every generated client implements the service trait itself, so each
//! scenario is one generic `async fn` over `AuthService` +
//! `OrganizationService` — no adapter in between.
//! * **vox, in-process** — the same `LayerRouter` over a memory link
//!   (`architect::LocalServer`), driven by the generated vox clients;
//! * **vox over WebSocket** — the same router behind `/vox` on a real
//!   socket, dialed through `architect::connect`;
//! * **vox over iroh** — the same router on an iroh endpoint, dialed
//!   peer to peer over loopback (relays and lookup off).
//!
//! A behaviour that holds on one and not another is a bug in the
//! framework's face, not in the auth engine — which is exactly the kind
//! of bug a per-transport hand-written test would never notice.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::too_many_lines,
    clippy::future_not_send
)]

use std::sync::Arc;

use architect::connect::{Endpoint, Pool};
use architect::iroh_link::{self, VOX_ALPN};
use architect::{LocalServer, Scope};
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_proto::{
    AuthFlowError, AuthService, AuthServiceClient, AuthServiceHttpClient, AuthSessionBundle,
    Invite, NewOrganization, OrganizationService, OrganizationServiceClient,
    OrganizationServiceHttpClient, SignInEmailPassword, SignUpEmailPassword,
};
use auth_server::{ServerConfig, server};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use vox_core::FromVoxLane as _;

const PASSWORD: &str = "correct horse battery staple";

// ── The transports ─────────────────────────────────────────────────────

/// One engine, four ways in. Each stack is a fresh in-memory database, so
/// scenarios never see each other's users. `A` / `O` are whichever
/// generated clients the transport hands back — every one of them
/// implements the trait, which is the whole point.
struct Stack<A, O> {
    name: &'static str,
    auth: A,
    orgs: O,
    /// Whatever must stay alive for the transport to keep working.
    _keep: Vec<Box<dyn std::any::Any + Send>>,
}

type HttpStack = Stack<AuthServiceHttpClient, OrganizationServiceHttpClient>;
type VoxStack = Stack<AuthServiceClient, OrganizationServiceClient>;

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: "http://127.0.0.1".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    }
}

async fn engine() -> (
    ServerConfig,
    architect_auth::ArchitectAuth<AuthSeaOrmStorage>,
) {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("migrate");
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    (config, auth)
}

/// The full app on a real TCP port; returns its base URL.
async fn spawn_app(
    config: &ServerConfig,
    auth: architect_auth::ArchitectAuth<AuthSeaOrmStorage>,
) -> String {
    let app = server::app_router(config, auth).expect("router builds");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("127.0.0.1:{}", addr.port())
}

async fn http_stack() -> HttpStack {
    let (config, auth) = engine().await;
    let base = format!("http://{}", spawn_app(&config, auth).await);
    let transport = architect::http::HttpClient::new(base);
    Stack {
        name: "http",
        auth: AuthServiceHttpClient::new(transport.clone()),
        orgs: OrganizationServiceHttpClient::new(transport),
        _keep: Vec::new(),
    }
}

async fn local_vox_stack() -> VoxStack {
    let (_config, auth) = engine().await;
    let scope = Scope::new();
    let local = LocalServer::serve(server::vox_router(auth), scope.clone());
    Stack {
        name: "vox (in-process)",
        auth: local.establish().await.expect("establish auth"),
        orgs: local.establish().await.expect("establish orgs"),
        _keep: vec![Box::new(scope), Box::new(local)],
    }
}

async fn websocket_vox_stack() -> VoxStack {
    let (config, auth) = engine().await;
    let addr = spawn_app(&config, auth).await;
    // A pool of our own rather than the process-global one: every stack
    // is a distinct server, and the global pool would happily hand a
    // second test the first test's socket.
    let pool = Pool::default();
    let endpoint = Endpoint::url(format!("ws://{addr}/vox"));
    Stack {
        name: "vox (websocket)",
        auth: pool.client(&endpoint).await.expect("dial /vox"),
        orgs: pool.client(&endpoint).await.expect("dial /vox"),
        _keep: vec![Box::new(pool)],
    }
}

async fn bind_iroh() -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
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

/// A lane client that keeps the raw caller — one iroh connection, many
/// typed clients over it.
#[derive(Clone)]
struct Lane {
    caller: vox_core::Caller,
    _connection: Option<vox_core::ConnectionHandle>,
}

impl vox_core::FromVoxLane for Lane {
    const SERVICE_NAME: &'static str = "surfaces";
    fn from_vox_lane(
        caller: vox_core::Caller,
        connection: Option<vox_core::ConnectionHandle>,
    ) -> Self {
        Self {
            caller,
            _connection: connection,
        }
    }
}

async fn iroh_vox_stack() -> VoxStack {
    let (_config, auth) = engine().await;
    let router = server::vox_router(auth);

    let server_endpoint = bind_iroh().await;
    let server_addr = direct_addr(&server_endpoint);
    let serving = tokio::spawn({
        let endpoint = server_endpoint.clone();
        async move { iroh_link::serve_router(&endpoint, router).await }
    });

    let client_endpoint = bind_iroh().await;
    let link = iroh_link::connect(&client_endpoint, server_addr)
        .await
        .expect("dial over iroh");
    let lane: Lane = vox_core::initiator_on(link)
        .establish()
        .await
        .expect("vox handshake over iroh");
    Stack {
        name: "vox (iroh)",
        auth: AuthServiceClient::from_vox_lane(lane.caller.clone(), None),
        orgs: OrganizationServiceClient::from_vox_lane(lane.caller.clone(), None),
        _keep: vec![
            Box::new(lane),
            Box::new(serving),
            Box::new(server_endpoint),
            Box::new(client_endpoint),
        ],
    }
}

/// Run one scenario over every transport, naming the transport on
/// failure. The scenario is generic over the traits; the four stacks
/// hand it four different client types.
async fn on_every_transport<F>(scenario: F)
where
    F: for<'a> AsyncScenario<'a>,
{
    async fn run(name: &str, fut: impl std::future::Future<Output = ()>) {
        futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(fut))
            .await
            .unwrap_or_else(|_| panic!("scenario failed over {name}"));
    }
    let s = http_stack().await;
    run(s.name, scenario.call(&s.auth, &s.orgs)).await;
    let s = local_vox_stack().await;
    run(s.name, scenario.call(&s.auth, &s.orgs)).await;
    let s = websocket_vox_stack().await;
    run(s.name, scenario.call(&s.auth, &s.orgs)).await;
    let s = iroh_vox_stack().await;
    run(s.name, scenario.call(&s.auth, &s.orgs)).await;
}

/// A scenario: an async fn generic over the two service traits. The
/// trait exists only because a generic `async fn` cannot be named as a
/// closure type; `scenario!` wraps one.
trait AsyncScenario<'a> {
    fn call<A: AuthService + Sync + 'a, O: OrganizationService + Sync + 'a>(
        &self,
        auth: &'a A,
        orgs: &'a O,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>;
}

macro_rules! scenario {
    ($name:ident) => {{
        struct S;
        impl<'a> AsyncScenario<'a> for S {
            fn call<A: AuthService + Sync + 'a, O: OrganizationService + Sync + 'a>(
                &self,
                auth: &'a A,
                orgs: &'a O,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>> {
                Box::pin($name(auth, orgs))
            }
        }
        S
    }};
}

fn sign_up_input(email: &str) -> SignUpEmailPassword {
    SignUpEmailPassword {
        email: email.into(),
        password: PASSWORD.into(),
        name: Some("Cody".into()),
        username: None,
        image: None,
        metadata_json: None,
        ip_address: None,
        user_agent: None,
    }
}

fn sign_in_input(email: &str, password: &str) -> SignInEmailPassword {
    SignInEmailPassword {
        email: email.into(),
        password: password.into(),
        ip_address: None,
        user_agent: None,
    }
}

async fn signed_up(auth: &impl AuthService, email: &str) -> AuthSessionBundle {
    auth.sign_up_email_password(sign_up_input(email))
        .await
        .expect("sign up")
}

// ── Sessions ───────────────────────────────────────────────────────────

async fn sign_up_session_refresh_and_sign_out<A: AuthService, O: OrganizationService>(
    auth: &A,
    _orgs: &O,
) {
    let bundle = signed_up(auth, "cody@example.com").await;
    assert_eq!(bundle.user.email.as_deref(), Some("cody@example.com"));
    assert_eq!(bundle.user.name.as_deref(), Some("Cody"));
    assert!(!bundle.token.is_empty(), "sign-up returns the raw token");

    let current = auth
        .current_session(bundle.token.clone())
        .await
        .expect("session");
    assert_eq!(current.user.id, bundle.user.id);
    assert_eq!(
        auth.whoami(bundle.token.clone()).await.expect("whoami").id,
        bundle.user.id
    );

    // Refresh rotates: new token works, old token is dead — and the
    // engine says so as `SessionExpired`, on every face alike.
    let refreshed = auth
        .refresh_session(bundle.token.clone())
        .await
        .expect("refresh");
    assert_ne!(refreshed.token, bundle.token);
    auth.current_session(refreshed.token.clone())
        .await
        .expect("new token works");
    assert_eq!(
        auth.current_session(bundle.token.clone())
            .await
            .unwrap_err(),
        AuthFlowError::SessionExpired
    );

    // Sign-out is idempotent and does not reveal existence.
    auth.sign_out(refreshed.token.clone())
        .await
        .expect("sign out");
    auth.sign_out(refreshed.token.clone())
        .await
        .expect("sign out again");
    auth.sign_out("never-a-token".into())
        .await
        .expect("sign out of nothing");
    // A revoked session reads as expired, like a rotated one.
    assert_eq!(
        auth.current_session(refreshed.token).await.unwrap_err(),
        AuthFlowError::SessionExpired
    );
}

#[tokio::test]
async fn sessions_round_trip() {
    on_every_transport(scenario!(sign_up_session_refresh_and_sign_out)).await;
}

async fn signing_in_again_restores_a_working_session<A: AuthService, O: OrganizationService>(
    auth: &A,
    _orgs: &O,
) {
    let first = signed_up(auth, "again@example.com").await;
    auth.sign_out(first.token).await.expect("sign out");
    let again = auth
        .sign_in_email_password(sign_in_input("again@example.com", PASSWORD))
        .await
        .expect("sign in");
    assert_eq!(again.user.id, first.user.id);
    auth.current_session(again.token)
        .await
        .expect("session works");
}

#[tokio::test]
async fn sign_in_again() {
    on_every_transport(scenario!(signing_in_again_restores_a_working_session)).await;
}

async fn wrong_password_is_indistinguishable_from_unknown_account<
    A: AuthService,
    O: OrganizationService,
>(
    auth: &A,
    _orgs: &O,
) {
    signed_up(auth, "real@example.com").await;
    let wrong = auth
        .sign_in_email_password(sign_in_input("real@example.com", "not-it"))
        .await
        .unwrap_err();
    let unknown = auth
        .sign_in_email_password(sign_in_input("nobody@example.com", PASSWORD))
        .await
        .unwrap_err();
    assert_eq!(wrong, AuthFlowError::InvalidCredentials);
    assert_eq!(unknown, AuthFlowError::InvalidCredentials);
}

#[tokio::test]
async fn wrong_password() {
    on_every_transport(scenario!(
        wrong_password_is_indistinguishable_from_unknown_account
    ))
    .await;
}

async fn a_session_needs_a_credential<A: AuthService, O: OrganizationService>(auth: &A, orgs: &O) {
    assert_eq!(
        auth.current_session(String::new()).await.unwrap_err(),
        AuthFlowError::InvalidCredentials
    );
    assert_eq!(
        orgs.list_organizations(String::new()).await.unwrap_err(),
        AuthFlowError::InvalidCredentials
    );
}

#[tokio::test]
async fn credential_required() {
    on_every_transport(scenario!(a_session_needs_a_credential)).await;
}

// ── Organizations ──────────────────────────────────────────────────────

fn new_org(name: &str, slug: &str) -> NewOrganization {
    NewOrganization {
        name: name.into(),
        slug: slug.into(),
        logo: None,
        metadata_json: None,
    }
}

async fn creating_an_org_puts_it_on_the_callers_list_with_a_role<
    A: AuthService,
    O: OrganizationService,
>(
    auth: &A,
    orgs: &O,
) {
    let me = signed_up(auth, "owner@example.com").await;
    assert!(
        orgs.list_organizations(me.token.clone())
            .await
            .expect("list")
            .is_empty()
    );

    let created = orgs
        .create_organization(me.token.clone(), new_org("Fast Track", "fast-track"))
        .await
        .expect("create");
    assert_eq!(created.organization.slug, "fast-track");
    assert_eq!(created.membership.user_id, me.user.id);
    assert_eq!(created.membership.role, "owner");

    let listed = orgs
        .list_organizations(me.token.clone())
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].organization.id, created.organization.id);
    assert_eq!(listed[0].membership.role, "owner");

    let fetched = orgs
        .get_organization(me.token.clone(), created.organization.id)
        .await
        .expect("get");
    assert_eq!(fetched.organization.name, "Fast Track");

    orgs.set_active_organization(me.token.clone(), created.organization.id)
        .await
        .expect("set active");
    let session = auth.current_session(me.token).await.expect("session");
    assert_eq!(
        session.session.active_organization_id,
        Some(created.organization.id)
    );
}

#[tokio::test]
async fn org_create_list_get_activate() {
    on_every_transport(scenario!(
        creating_an_org_puts_it_on_the_callers_list_with_a_role
    ))
    .await;
}

async fn another_accounts_org_is_neither_listed_nor_readable<
    A: AuthService,
    O: OrganizationService,
>(
    auth: &A,
    orgs: &O,
) {
    let owner = signed_up(auth, "owner@example.com").await;
    let other = signed_up(auth, "other@example.com").await;
    let org = orgs
        .create_organization(owner.token, new_org("Private", "private"))
        .await
        .expect("create");

    assert!(
        orgs.list_organizations(other.token.clone())
            .await
            .expect("list")
            .is_empty()
    );
    let refused = orgs
        .get_organization(other.token, org.organization.id)
        .await
        .unwrap_err();
    // The engine's choice: a non-member gets the same answer as a
    // missing org, and both faces carry it through unchanged.
    assert!(
        matches!(
            refused,
            AuthFlowError::PermissionDenied | AuthFlowError::InvalidInput(_)
        ),
        "{refused:?}"
    );
}

#[tokio::test]
async fn org_isolation() {
    on_every_transport(scenario!(
        another_accounts_org_is_neither_listed_nor_readable
    ))
    .await;
}

async fn an_invitation_puts_the_invitee_in_the_org_and_roles_change<
    A: AuthService,
    O: OrganizationService,
>(
    auth: &A,
    orgs: &O,
) {
    let owner = signed_up(auth, "owner@example.com").await;
    let invitee = signed_up(auth, "invitee@example.com").await;
    let org = orgs
        .create_organization(owner.token.clone(), new_org("Team", "team"))
        .await
        .expect("create");

    let issued = orgs
        .invite_member(
            owner.token.clone(),
            Invite {
                organization_id: org.organization.id,
                email: "invitee@example.com".into(),
                role: "member".into(),
                expires_at: None,
            },
        )
        .await
        .expect("invite");
    assert_eq!(issued.invitation.email, "invitee@example.com");
    assert!(
        !issued.token.is_empty(),
        "the redeem token is returned once"
    );

    orgs.accept_invitation(invitee.token.clone(), issued.invitation.id, issued.token)
        .await
        .expect("accept");

    let members = orgs
        .list_members(owner.token.clone(), org.organization.id)
        .await
        .expect("members");
    assert_eq!(members.len(), 2, "{members:?}");
    let joined = members
        .iter()
        .find(|m| m.user.id == invitee.user.id)
        .expect("invitee is a member");
    assert_eq!(joined.member.role, "member");
    assert_eq!(joined.user.email.as_deref(), Some("invitee@example.com"));

    let promoted = orgs
        .update_member_role(
            owner.token.clone(),
            org.organization.id,
            invitee.user.id,
            "admin".into(),
        )
        .await
        .expect("promote");
    assert_eq!(promoted.role, "admin");
    let members = orgs
        .list_members(owner.token, org.organization.id)
        .await
        .expect("members");
    let joined = members
        .iter()
        .find(|m| m.user.id == invitee.user.id)
        .unwrap();
    assert_eq!(joined.member.role, "admin");

    // The invitee now sees the org on their own list.
    let mine = orgs.list_organizations(invitee.token).await.expect("list");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].membership.role, "admin");
}

#[tokio::test]
async fn org_invitations_and_roles() {
    on_every_transport(scenario!(
        an_invitation_puts_the_invitee_in_the_org_and_roles_change
    ))
    .await;
}

// ── The HTTP face's own affordances ────────────────────────────────────

/// Over HTTP the session token may travel as `Authorization: Bearer`
/// instead of in the body — the generated router fills the `token`
/// argument in. A browser keeps the credential in a header, as it does
/// for every other API.
#[tokio::test]
async fn http_accepts_the_token_as_a_bearer_header() {
    let (config, auth) = engine().await;
    let base = format!("http://{}", spawn_app(&config, auth).await);
    let anonymous = AuthServiceHttpClient::at(base.clone());
    let bundle = AuthServiceHttpClient::sign_up_email_password(
        &anonymous,
        sign_up_input("bearer@example.com"),
    )
    .await
    .expect("sign up");

    let as_bearer = AuthServiceHttpClient::at(base).with_bearer(bundle.token.clone());
    // Empty body token: the header carries it.
    let me = as_bearer
        .whoami(String::new())
        .await
        .expect("whoami via bearer");
    assert_eq!(me.id, bundle.user.id);
    // The body wins when it says something — even something wrong.
    assert_eq!(
        as_bearer.whoami("bogus".into()).await.unwrap_err().app(),
        Some(&AuthFlowError::InvalidCredentials)
    );
}

/// The error envelope on the wire: status from `HttpError`, a stable
/// `code`, and the typed error itself — what a non-Rust client reads.
#[tokio::test]
async fn http_errors_carry_status_code_and_the_typed_error() {
    let (config, auth) = engine().await;
    let base = format!("http://{}", spawn_app(&config, auth).await);
    let response = reqwest::Client::new()
        .post(format!("{base}/auth/current-session"))
        .header("content-type", "application/json")
        .body(r#"{"token":"nope"}"#)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status().as_u16(), 401);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["code"], "invalid_credentials");
    assert_eq!(body["message"], "invalid credentials");
    assert_eq!(body["error"], "InvalidCredentials");
}

/// Every route the two services generate is mounted, and nothing that
/// used to be hand-written next to them survives.
#[tokio::test]
async fn the_generated_paths_are_the_whole_json_api() {
    let (config, auth) = engine().await;
    let base = format!("http://{}", spawn_app(&config, auth).await);
    let http = reqwest::Client::new();
    for (_, path) in auth_proto::service::http::PATHS
        .iter()
        .chain(auth_proto::organizations::http::PATHS)
    {
        let status = http
            .post(format!("{base}{path}"))
            .send()
            .await
            .expect("request")
            .status();
        assert_ne!(status.as_u16(), 404, "{path} is mounted");
        assert_ne!(status.as_u16(), 405, "{path} takes POST");
    }
    for old in [
        "/auth/sign-in/email",
        "/auth/session",
        "/auth/organization/list",
    ] {
        let status = http
            .post(format!("{base}{old}"))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status.as_u16(), 404, "{old} is gone");
    }
}

/// The process-global pool, as an app would use it: one socket, both
/// clients, the bearer riding the WebSocket handshake.
#[tokio::test]
async fn one_pooled_websocket_serves_every_client() {
    let (config, auth) = engine().await;
    let addr = spawn_app(&config, auth).await;
    let endpoint = Endpoint::url(format!("ws://{addr}/vox"));
    let pool = Arc::new(Pool::default());
    let auth_client: AuthServiceClient = pool.client(&endpoint).await.expect("dial");
    let org_client: OrganizationServiceClient = pool.client(&endpoint).await.expect("dial");
    let bundle = AuthServiceClient::sign_up_email_password(
        &auth_client,
        sign_up_input("pooled@example.com"),
    )
    .await
    .expect("sign up");
    let orgs = OrganizationServiceClient::list_organizations(&org_client, bundle.token)
        .await
        .expect("list");
    assert!(orgs.is_empty());
}
