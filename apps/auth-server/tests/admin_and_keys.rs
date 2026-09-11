//! The operator screen and the API-key page.
//!
//! The assertions that matter here are the refusals. `/admin/users` is
//! every account on the server; a signed-in stranger reaching it is the
//! whole ballgame.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::string_slice
)]

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: "https://auth.fasttrackstudio.app".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    }
}

/// A server with the seeded cast, so there is an administrator.
async fn app() -> (axum::Router, DatabaseConnection) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db.clone())).unwrap();
    auth_server::dev::seed(&auth, "sqlite::memory:")
        .await
        .unwrap();
    let app = server::app_router(&config, auth).unwrap();
    (app, db)
}

async fn sign_in(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/sign-in/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"email":"{email}","password":"{}"}}"#,
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

async fn get(app: &axum::Router, uri: &str, token: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder().uri(uri);
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

async fn post(
    app: &axum::Router,
    uri: &str,
    token: Option<&str>,
    form: &str,
) -> (StatusCode, String, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(form.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn value_of_input_labelled(html: &str, label: &str) -> String {
    let marker = format!(r#"aria-label="{label}""#);
    let at = html.find(&marker).expect("an input with that label");
    let before = html.get(..at).unwrap();
    let value_at = before.rfind(r#"value=""#).expect("a value attribute");
    before
        .get(value_at + r#"value=""#.len()..)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned()
}

// ── Admin ────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_operator_screen_is_shut_to_everybody_but_an_administrator() {
    let (app, _) = app().await;

    let (anonymous, _) = get(&app, "/admin/users", None).await;
    assert_eq!(
        anonymous,
        StatusCode::SEE_OTHER,
        "signed out goes to sign in"
    );

    // Signed in, not an admin. This is the one that matters: the page
    // is every account on the server.
    let grace = sign_in(&app, "grace@local.test").await;
    let (forbidden, body) = get(&app, "/admin/users", Some(&grace)).await;
    assert_eq!(forbidden, StatusCode::FORBIDDEN);
    assert!(body.contains("for server administrators"), "{body:.300}");
    // And it must not have leaked the list on the way to refusing.
    assert!(!body.contains("alan@local.test"), "{body:.400}");

    let ada = sign_in(&app, "ada@local.test").await;
    let (ok, body) = get(&app, "/admin/users", Some(&ada)).await;
    assert_eq!(ok, StatusCode::OK);
    assert!(body.contains("ada@local.test"));
    assert!(body.contains("grace@local.test"));
}

#[tokio::test]
async fn a_non_administrator_cannot_ban_anybody_by_posting_directly() {
    let (app, _) = app().await;
    let grace = sign_in(&app, "grace@local.test").await;
    let ada = sign_in(&app, "ada@local.test").await;

    // The list page is one gate; every action carries its own, because
    // a hidden button is not a control.
    let (_, page) = get(&app, "/admin/users", Some(&ada)).await;
    let alan_id = page
        .split("alan@local.test")
        .next()
        .and_then(|before| {
            before
                .rfind(r#"name="user_id" value=""#)
                .map(|at| (before, at))
        })
        .map(|(before, at)| {
            before[at + r#"name="user_id" value=""#.len()..]
                .split('"')
                .next()
                .unwrap()
                .to_owned()
        })
        .expect("a user id on the page");

    let (_, location, _) = post(
        &app,
        "/admin/users/ban",
        Some(&grace),
        &format!("user_id={alan_id}&reason=because"),
    )
    .await;
    assert!(location.contains("error="), "{location}");

    // Still not banned.
    let (_, page) = get(&app, "/admin/users", Some(&ada)).await;
    let alan_row = page
        .split("alan@local.test")
        .nth(1)
        .unwrap_or_default()
        .chars()
        .take(400)
        .collect::<String>();
    assert!(!alan_row.contains("banned"), "{alan_row}");
}

#[tokio::test]
async fn banning_shows_on_the_list_and_unbanning_takes_it_off() {
    let (app, _) = app().await;
    let ada = sign_in(&app, "ada@local.test").await;
    let (_, page) = get(&app, "/admin/users", Some(&ada)).await;
    let before = page.split("alan@local.test").next().unwrap();
    let at = before.rfind(r#"name="user_id" value=""#).unwrap();
    let alan_id = before[at + r#"name="user_id" value=""#.len()..]
        .split('"')
        .next()
        .unwrap()
        .to_owned();

    let (_, location, _) = post(
        &app,
        "/admin/users/ban",
        Some(&ada),
        &format!("user_id={alan_id}&reason=spam"),
    )
    .await;
    assert!(location.contains("ok="), "{location}");
    let (_, page) = get(&app, "/admin/users", Some(&ada)).await;
    assert!(page.contains("spam"), "the reason should be visible");

    let (_, location, _) = post(
        &app,
        "/admin/users/unban",
        Some(&ada),
        &format!("user_id={alan_id}"),
    )
    .await;
    assert!(location.contains("ok="), "{location}");
}

// ── API keys ─────────────────────────────────────────────────────────

#[tokio::test]
async fn a_key_is_shown_once_and_then_only_by_its_prefix() {
    let (app, _) = app().await;
    let grace = sign_in(&app, "grace@local.test").await;

    let (status, _) = get(&app, "/account/api-keys", Some(&grace)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, page) = post(
        &app,
        "/account/api-keys",
        Some(&grace),
        "name=CI+deploys&expires_days=",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "creation renders its own result");
    let key = value_of_input_labelled(&page, "New API key");
    assert!(key.starts_with("ak_"), "{key}");
    assert!(key.len() > 20, "{key}");

    // On a later visit only the prefix survives — the key itself is
    // stored as a hash and cannot be shown again.
    let (_, page) = get(&app, "/account/api-keys", Some(&grace)).await;
    assert!(page.contains("CI deploys"));
    assert!(
        !page.contains(&key),
        "the whole key must not be re-rendered"
    );
    assert!(page.contains(&key[..12]), "the prefix should be");
}

#[tokio::test]
async fn revoking_keeps_the_record_and_deleting_removes_it() {
    let (app, _) = app().await;
    let grace = sign_in(&app, "grace@local.test").await;
    let (_, _, page) = post(
        &app,
        "/account/api-keys",
        Some(&grace),
        "name=Leaked&expires_days=",
    )
    .await;
    let at = page.find(r#"name="id" value=""#).expect("a key id");
    let id = page[at + r#"name="id" value=""#.len()..]
        .split('"')
        .next()
        .unwrap()
        .to_owned();

    let (_, location, _) = post(
        &app,
        "/account/api-keys/revoke",
        Some(&grace),
        &format!("id={id}"),
    )
    .await;
    assert!(location.contains("ok="), "{location}");

    // Still listed, marked off. Somebody responding to a leak needs the
    // record to survive the response.
    let (_, page) = get(&app, "/account/api-keys", Some(&grace)).await;
    assert!(page.contains("Leaked"));
    assert!(page.contains("revoked"), "{page:.700}");

    let (_, location, _) = post(
        &app,
        "/account/api-keys/delete",
        Some(&grace),
        &format!("id={id}"),
    )
    .await;
    assert!(location.contains("ok="), "{location}");
    let (_, page) = get(&app, "/account/api-keys", Some(&grace)).await;
    assert!(!page.contains("Leaked"), "{page:.700}");
}

#[tokio::test]
async fn one_persons_keys_are_not_another_persons() {
    let (app, _) = app().await;
    let grace = sign_in(&app, "grace@local.test").await;
    let alan = sign_in(&app, "alan@local.test").await;

    post(
        &app,
        "/account/api-keys",
        Some(&grace),
        "name=Graces+key&expires_days=",
    )
    .await;

    let (_, page) = get(&app, "/account/api-keys", Some(&alan)).await;
    assert!(!page.contains("Graces key"), "{page:.600}");
    assert!(page.contains("No keys yet"), "{page:.600}");
}
