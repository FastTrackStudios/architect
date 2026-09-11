//! The two local-server workflows, end to end.
//!
//! The assertions that matter are about who is refused. A snapshot is
//! the entire organization graph; "who is in which company" is not
//! public, and a signed-in stranger must not be able to fetch it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

fn config(database_url: &str) -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: database_url.into(),
        base_url: "http://localhost:8080".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    }
}

/// A seeded local server, and the database behind it.
async fn seeded() -> (axum::Router, DatabaseConnection) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = config("sqlite::memory:");
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db.clone())).unwrap();
    let seeded = auth_server::dev::seed(&auth, &config.database_url)
        .await
        .expect("seed");
    assert!(seeded, "a fresh database should seed");
    let app = server::app_router_with_db(&config, auth, db.clone()).unwrap();
    (app, db)
}

async fn sign_in(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/sign-in-email-password")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"input":{{"email":"{email}","password":"{}"}}}}"#,
                    auth_server::dev::DEV_PASSWORD
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "sign in {email}");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["token"].as_str().unwrap().to_owned()
}

async fn snapshot_as(app: &axum::Router, token: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder().uri("/admin/snapshot");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn a_fresh_server_comes_up_with_people_and_organizations_in_it() {
    let (app, _) = seeded().await;
    let ada = sign_in(&app, "ada@local.test").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/orgs")
                .header(header::AUTHORIZATION, format!("Bearer {ada}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let page = String::from_utf8_lossy(&bytes);
    assert!(page.contains("Acme Records"), "{page:.600}");
}

#[tokio::test]
async fn seeding_twice_does_not_duplicate_anybody() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = config("sqlite::memory:");
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).unwrap();

    assert!(
        auth_server::dev::seed(&auth, "sqlite::memory:")
            .await
            .unwrap()
    );
    // A restarted dev server must not fail on a unique constraint.
    assert!(
        !auth_server::dev::seed(&auth, "sqlite::memory:")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn seeding_refuses_a_database_that_is_not_local() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = config("sqlite::memory:");
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).unwrap();

    // The guard: seeding creates accounts whose password is published
    // in this repository.
    let refused = auth_server::dev::seed(&auth, "postgres://a:pw@auth.prod.internal/auth").await;
    let message = refused.unwrap_err().to_string();
    assert!(message.contains("refusing to seed"), "{message}");
    assert!(!message.contains("pw@"), "{message}");
}

/// The one that matters: a mirror you cannot sign in to is not a mirror.
///
/// This exists because an earlier version wrote the *user id* into the
/// credential account's `account_id`, where the email/password sign-in
/// path looks for the address. Every table was populated, every row
/// count was right, and nobody could log in — a shape no
/// assertion-on-rows would have caught.
#[tokio::test]
async fn everyone_in_an_imported_snapshot_can_actually_sign_in() {
    let (source_app, source_db) = seeded().await;
    let ada = sign_in(&source_app, "ada@local.test").await;
    let (status, body) = snapshot_as(&source_app, Some(&ada)).await;
    assert_eq!(status, StatusCode::OK);
    drop(source_db);

    let snapshot: architect_auth::db::snapshot::Snapshot = serde_json::from_str(&body).unwrap();

    // A different secret, as a real local server would have: password
    // verification must not depend on it.
    let target_db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&target_db, None).await.unwrap();
    architect_auth::db::snapshot::import(
        &target_db,
        "sqlite::memory:",
        &snapshot,
        &auth_server::dev::dev_password_hash().unwrap(),
    )
    .await
    .unwrap();

    let mut config = config("sqlite::memory:");
    config.secret = "a-completely-different-secret-32b".into();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(target_db.clone())).unwrap();
    let app = server::app_router_with_db(&config, auth, target_db).unwrap();

    for (email, _, _) in auth_server::dev::DEV_PEOPLE {
        let token = sign_in(&app, email).await;
        assert!(!token.is_empty(), "{email} could not sign in");
    }

    // And the graph they signed in to is the one that came across.
    let grace = sign_in(&app, "grace@local.test").await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/orgs")
                .header(header::AUTHORIZATION, format!("Bearer {grace}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let page = String::from_utf8_lossy(&bytes);
    assert!(page.contains("Indie Collective"), "{page:.600}");
    assert!(page.contains("Acme Records"), "{page:.600}");
}

#[tokio::test]
async fn only_an_administrator_can_take_a_snapshot() {
    let (app, _) = seeded().await;

    let (anonymous, _) = snapshot_as(&app, None).await;
    assert_eq!(anonymous, StatusCode::UNAUTHORIZED);

    // Signed in, but not an admin. This is the one that would leak the
    // whole graph if the gate were only "is there a session".
    let grace = sign_in(&app, "grace@local.test").await;
    let (forbidden, _) = snapshot_as(&app, Some(&grace)).await;
    assert_eq!(forbidden, StatusCode::FORBIDDEN);

    let (nonsense, _) = snapshot_as(&app, Some("not-a-token")).await;
    assert_eq!(nonsense, StatusCode::FORBIDDEN);

    let ada = sign_in(&app, "ada@local.test").await;
    let (ok, body) = snapshot_as(&app, Some(&ada)).await;
    assert_eq!(ok, StatusCode::OK);
    assert!(body.contains("Acme Records"), "{body:.400}");
}

#[tokio::test]
async fn a_snapshot_over_http_carries_no_password_hash() {
    let (app, _) = seeded().await;
    let ada = sign_in(&app, "ada@local.test").await;
    let (status, body) = snapshot_as(&app, Some(&ada)).await;
    assert_eq!(status, StatusCode::OK);

    // The seeded accounts have real Argon2 hashes behind them. None of
    // them, nor the marker of one, may appear in the response.
    assert!(!body.contains("$argon2"), "a password hash was served");
    assert!(!body.contains("password_hash"), "{body:.400}");
    assert!(!body.contains("token_hash"), "{body:.400}");
    // And it is a real snapshot, not an empty one that trivially passes.
    assert!(body.contains("ada@local.test"));
}

#[tokio::test]
async fn a_redacted_snapshot_keeps_the_graph_and_drops_the_addresses() {
    let (app, _) = seeded().await;
    let ada = sign_in(&app, "ada@local.test").await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/snapshot?redact_emails=true")
                .header(header::AUTHORIZATION, format!("Bearer {ada}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&bytes);

    assert!(!body.contains("ada@local.test"), "{body:.400}");
    assert!(body.contains("@local.invalid"), "{body:.400}");
    assert!(body.contains("Acme Records"), "{body:.400}");
}
