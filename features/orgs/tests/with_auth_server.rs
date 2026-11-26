//! architect-orgs against the real identity server (auth-server, in this
//! process): the orgs every app reads from one sign-in, a confining
//! server's directory, and architect's connection pool serving every
//! client over one WebSocket.
#![cfg(all(feature = "auth", feature = "server"))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use architect::connect::{Endpoint, Pool};
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use architect_auth::vox_host::local;
use auth_proto::{
    AuthService, AuthServiceClient, AuthServiceHttpClient, AuthSessionBundle, NewOrganization,
    OrganizationService, OrganizationServiceClient, OrganizationServiceHttpClient,
    SignUpEmailPassword,
};
use auth_server::{ServerConfig, server};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

const PASSWORD: &str = "correct horse battery staple";

async fn engine() -> (
    ServerConfig,
    architect_auth::ArchitectAuth<AuthSeaOrmStorage>,
) {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("migrate");
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: "http://127.0.0.1".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    };
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    (config, auth)
}

/// The full app on a real TCP port; its `host:port`.
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

async fn signed_up(auth: &impl AuthService, email: &str) -> AuthSessionBundle {
    auth.sign_up_email_password(sign_up_input(email))
        .await
        .expect("sign up")
}

fn new_org(name: &str, slug: &str) -> NewOrganization {
    NewOrganization {
        name: name.into(),
        slug: slug.into(),
        logo: None,
        metadata_json: None,
    }
}

/// Two apps signed in with the one session show the same orgs, with the
/// person's role in each; signed out, the fetch says so.
async fn every_app_reads_the_same_orgs_from_one_sign_in<A: AuthService, O: OrganizationService>(
    auth: &A,
    orgs: &O,
) {
    use architect_orgs::{Orgs, Role};

    let me = signed_up(auth, "owner@example.com").await;
    let band = orgs
        .create_organization(me.token.clone(), new_org("The Band", "the-band"))
        .await
        .expect("create");
    let session =
        auth_client::StoredSession::new(me.token.clone()).with_user_id(me.user.id.to_string());

    let task = Orgs::new(architect_orgs::fetch(orgs, &session).await.expect("fetch"));
    let studio = Orgs::new(architect_orgs::fetch(orgs, &session).await.expect("fetch"));
    assert_eq!(task, studio);
    let org = task.current().expect("one org, selected");
    assert_eq!(org.id.0, band.organization.id.to_string());
    assert_eq!(org.name, "The Band");
    assert_eq!(org.role, Role::Owner);

    auth.sign_out(me.token).await.expect("sign out");
    assert!(architect_orgs::fetch(orgs, &session).await.is_err());
}

#[tokio::test]
async fn orgs_fetched_live_over_http() {
    let (config, auth) = engine().await;
    let base = format!("http://{}", spawn_app(&config, auth).await);
    let transport = auth_proto::http::HttpClient::new(base);
    every_app_reads_the_same_orgs_from_one_sign_in(
        &AuthServiceHttpClient::new(transport.clone()),
        &OrganizationServiceHttpClient::new(transport),
    )
    .await;
}

#[tokio::test]
async fn orgs_fetched_live_over_vox() {
    let (_config, auth) = engine().await;
    let router = server::vox_router(auth);
    let (a, _sa): (AuthServiceClient, _) = local::establish(&router).await.expect("auth");
    let (o, _so): (OrganizationServiceClient, _) = local::establish(&router).await.expect("orgs");
    every_app_reads_the_same_orgs_from_one_sign_in(&a, &o).await;
}

/// A server confining its services to the caller's org learns the same
/// orgs from the caller's token as the apps do.
#[tokio::test]
async fn a_confining_server_learns_the_callers_orgs_from_the_identity_server() {
    use architect_orgs::server::{Directory as _, IdentityServer};

    let (_config, auth) = engine().await;
    let router = server::vox_router(auth);
    let (a, _sa): (AuthServiceClient, _) = local::establish(&router).await.expect("auth");
    let (o, _so): (OrganizationServiceClient, _) = local::establish(&router).await.expect("orgs");
    let me = signed_up(&a, "owner@example.com").await;
    o.create_organization(me.token.clone(), new_org("The Band", "the-band"))
        .await
        .expect("create");
    let session = auth_client::StoredSession::new(me.token.clone());
    let apps = architect_orgs::fetch(&o, &session).await.expect("fetch");
    let directory = IdentityServer(o.clone());
    assert_eq!(directory.orgs(&me.token).await.expect("directory"), apps);
    assert!(directory.orgs("not-a-session").await.is_err());
}

/// architect's connection pool, as an app uses it: one socket to the
/// identity server's `/vox`, both clients over it.
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
