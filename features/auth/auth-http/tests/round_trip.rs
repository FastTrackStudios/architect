//! Drive `auth-http` against a real `auth-server` over a real socket.
//!
//! The unit tests cover shaping; this covers the thing that actually
//! matters — that the JSON this client expects is the JSON the server
//! emits. A mismatch between the two is invisible to both crates'
//! own test suites, and would only show up in a browser.

use std::sync::Arc;

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_client::{MemoryTokenStore, TokenStore};
use auth_http::{AuthHttpClient, SignUpRequest};
use auth_server::{ServerConfig, server};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

/// Start a server on an ephemeral port and return its base URL.
async fn spawn_server() -> String {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    Migrator::up(&db, None).await.expect("migrate");

    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        secret: "a-secret-at-least-32-bytes-long!!".into(),
        // http, so the cookie is not marked Secure — irrelevant here
        // (this client uses bearer tokens) but it keeps the server's
        // own logging honest about what it is serving.
        base_url: "http://127.0.0.1".into(),
        oidc_issuer: None,
        session_ttl_seconds: 3600,
        require_email_verification: false,
        passkey_rp_id: None,
        cors_origins: Vec::new(),
        oidc_clients: Vec::new(),
        oidc_allow_dynamic_client_registration: false,
        run_migrations: false,
    };

    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    let app = server::app_router(&config, auth);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn sign_up_session_refresh_and_sign_out_round_trip() {
    let base = spawn_server().await;
    let store = Arc::new(MemoryTokenStore::new());
    let client = AuthHttpClient::new(&base).with_store(store.clone());

    assert!(!client.has_token(), "a fresh client holds no token");

    // Sign up mints a token and the client remembers it.
    let signed_up = client
        .sign_up(
            &SignUpRequest::new("cody@fasttrackstudio.app", "correct-horse-battery-staple")
                .with_name("Cody"),
        )
        .await
        .expect("sign up");
    assert_eq!(
        signed_up.user.email.as_deref(),
        Some("cody@fasttrackstudio.app")
    );
    assert_eq!(signed_up.user.name.as_deref(), Some("Cody"));
    assert!(client.has_token(), "sign-up persists the token");

    // The stored token resolves back to the same user.
    let current = client.session().await.expect("session");
    assert_eq!(current.user.id, signed_up.user.id);
    // GET /auth/session deliberately does not echo the token.
    assert!(current.token.is_none());

    // Refresh rotates the token — and the client must have picked up
    // the new one, or the next call would use a revoked token.
    let before = client.token().expect("token before refresh");
    let refreshed = client.refresh().await.expect("refresh");
    let after = client.token().expect("token after refresh");
    assert_eq!(refreshed.user.id, signed_up.user.id);
    assert_ne!(before, after, "refresh must rotate the token");
    client.session().await.expect("session after refresh");

    // Sign out revokes it and forgets it locally.
    client.sign_out().await.expect("sign out");
    assert!(!client.has_token());
    assert!(store.load().expect("load").is_none());
}

#[tokio::test]
async fn signing_in_again_restores_a_working_session() {
    let base = spawn_server().await;
    let client =
        AuthHttpClient::new(&base).with_store(Arc::new(MemoryTokenStore::new()));

    client
        .sign_up(&SignUpRequest::new(
            "cody@fasttrackstudio.app",
            "correct-horse-battery-staple",
        ))
        .await
        .expect("sign up");
    client.sign_out().await.expect("sign out");

    let signed_in = client
        .sign_in("cody@fasttrackstudio.app", "correct-horse-battery-staple")
        .await
        .expect("sign in");
    assert_eq!(
        signed_in.user.email.as_deref(),
        Some("cody@fasttrackstudio.app")
    );
    client.session().await.expect("session works after sign-in");
}

#[tokio::test]
async fn wrong_password_surfaces_as_an_unauthenticated_api_error() {
    let base = spawn_server().await;
    let client =
        AuthHttpClient::new(&base).with_store(Arc::new(MemoryTokenStore::new()));

    client
        .sign_up(&SignUpRequest::new(
            "cody@fasttrackstudio.app",
            "correct-horse-battery-staple",
        ))
        .await
        .expect("sign up");

    let error = client
        .sign_in("cody@fasttrackstudio.app", "wrong")
        .await
        .expect_err("wrong password must fail");
    assert!(
        error.is_unauthenticated(),
        "a UI routes on this; got {error:?}"
    );
}

#[tokio::test]
async fn session_without_a_token_fails_locally_without_a_request() {
    let base = spawn_server().await;
    let client = AuthHttpClient::new(&base);

    let error = client.session().await.expect_err("no token");
    assert!(matches!(error, auth_http::AuthHttpError::NoToken));
    assert!(error.is_unauthenticated());
}

#[tokio::test]
async fn sign_out_clears_the_token_even_when_the_server_is_gone() {
    // The failure mode this guards: a user taps "sign out" on a flaky
    // connection, the request fails, and they are left looking signed
    // in on a device they meant to leave.
    let store = Arc::new(MemoryTokenStore::new());
    let base = spawn_server().await;
    let client = AuthHttpClient::new(&base).with_store(store.clone());
    client
        .sign_up(&SignUpRequest::new(
            "cody@fasttrackstudio.app",
            "correct-horse-battery-staple",
        ))
        .await
        .expect("sign up");

    // Point the client at a dead address, keeping the same store.
    let offline = AuthHttpClient::new("http://127.0.0.1:1").with_store(store.clone());
    assert!(offline.has_token());
    let _ = offline.sign_out().await;
    assert!(
        store.load().expect("load").is_none(),
        "the local token must be cleared regardless"
    );
}
