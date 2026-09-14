//! The passkey pages, driven the way the browser script drives them.
//!
//! The script is the only JavaScript in this UI, and these tests stand
//! in for it: they make exactly the same four JSON calls in the same
//! order, with a software authenticator doing what a phone would. So
//! everything below the script is covered without a browser, and the
//! browser suite only has to check that the buttons are wired.

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
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;

const ORIGIN: &str = "http://localhost:8080";

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: ORIGIN.into(),
        session_ttl_seconds: 3600,
        passkey_rp_id: Some("localhost".into()),
        ..ServerConfig::local()
    }
}

async fn app() -> axum::Router {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).unwrap();
    server::app_router(&config, auth).unwrap()
}

async fn signed_up(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/sign-up/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"input":{{"email":"{email}","password":"correct horse battery staple"}}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["token"].as_str().unwrap().to_owned()
}

/// A JSON POST, as the script makes it.
async fn post_json(
    app: &axum::Router,
    uri: &str,
    token: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json, set_cookie)
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

fn authenticator() -> WebauthnAuthenticator<SoftPasskey> {
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

fn origin() -> url::Url {
    url::Url::parse(ORIGIN).unwrap()
}

/// Register a passkey through the same two endpoints the script uses.
async fn register(
    app: &axum::Router,
    authenticator: &mut WebauthnAuthenticator<SoftPasskey>,
    token: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    let (status, start, _) = post_json(
        app,
        "/account/passkeys/begin",
        Some(token),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "begin: {start}");

    let options = serde_json::from_str(start["options"].as_str().unwrap()).unwrap();
    let credential = authenticator.do_registration(origin(), options).unwrap();

    post_json(
        app,
        "/account/passkeys/complete",
        Some(token),
        serde_json::json!({
            "handle": start["handle"],
            "name": name,
            "credential": serde_json::to_value(&credential).unwrap(),
        }),
    )
    .await
    .into_iter_first_two()
}

/// Small helper so the tuple above reads as `(status, body)`.
trait FirstTwo {
    fn into_iter_first_two(self) -> (StatusCode, serde_json::Value);
}
impl FirstTwo for (StatusCode, serde_json::Value, String) {
    fn into_iter_first_two(self) -> (StatusCode, serde_json::Value) {
        (self.0, self.1)
    }
}

#[tokio::test]
async fn a_passkey_is_registered_and_then_signs_in_over_http() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let mut authenticator = authenticator();

    let (status, body) = register(&app, &mut authenticator, &token, "My phone").await;
    assert_eq!(status, StatusCode::OK, "complete: {body}");
    assert_eq!(body["name"], "My phone");

    let (_, page) = get(&app, "/account/passkeys", Some(&token)).await;
    assert!(page.contains("My phone"), "{page:.600}");
    // The controls that need the API are hidden until the script says
    // otherwise, so a browser without it sees no dead buttons.
    assert!(page.contains(r#"data-passkey="true""#), "{page:.600}");

    // Sign in, exactly as the script does: begin, sign, complete.
    let (status, start, _) = post_json(
        &app,
        "/login/passkey/begin",
        None,
        serde_json::json!({ "email": "ada@example.com" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{start}");
    let options = serde_json::from_str(start["options"].as_str().unwrap()).unwrap();
    let assertion = authenticator.do_authentication(origin(), options).unwrap();

    let (status, done, set_cookie) = post_json(
        &app,
        "/login/passkey/complete",
        None,
        serde_json::json!({
            "handle": start["handle"],
            "return_to": "/orgs",
            "credential": serde_json::to_value(&assertion).unwrap(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["redirect"], "/orgs");
    assert!(set_cookie.contains('='), "a session cookie must be set");

    // And the cookie really is a working session.
    let session = set_cookie
        .split(';')
        .next()
        .and_then(|c| c.split_once('='))
        .map(|(_, value)| value.to_owned())
        .unwrap();
    let (status, page) = get(&app, "/orgs", Some(&session)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Organizations"), "{page:.300}");
}

#[tokio::test]
async fn a_forged_assertion_is_refused_with_401() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let mut authenticator = authenticator();
    register(&app, &mut authenticator, &token, "phone").await;

    let (_, start, _) = post_json(
        &app,
        "/login/passkey/begin",
        None,
        serde_json::json!({ "email": "ada@example.com" }),
    )
    .await;

    // The old bypass, over HTTP: a credential id, a challenge, and no
    // signature worth the name.
    let (status, body, set_cookie) = post_json(
        &app,
        "/login/passkey/complete",
        None,
        serde_json::json!({
            "handle": start["handle"],
            "credential": {
                "id": "AAAA",
                "rawId": "AAAA",
                "type": "public-key",
                "extensions": {},
                "response": {
                    "authenticatorData": "SZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2MFAAAAAQ",
                    "clientDataJSON": "e30",
                    "signature": "MEUCIQD",
                    "userHandle": null
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(set_cookie.is_empty(), "no session may be handed out");
}

#[tokio::test]
async fn beginning_a_sign_in_says_the_same_thing_about_everybody() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let mut authenticator = authenticator();
    register(&app, &mut authenticator, &token, "phone").await;

    // A registered address, an unknown one, and nonsense: all 200, all
    // carrying a challenge. Anything else is an account-enumeration
    // oracle on an unauthenticated endpoint.
    for email in [
        serde_json::json!("ada@example.com"),
        serde_json::json!("nobody@example.com"),
        serde_json::json!("not-an-address"),
        serde_json::Value::Null,
    ] {
        let (status, body, _) = post_json(
            &app,
            "/login/passkey/begin",
            None,
            serde_json::json!({ "email": email }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{email}: {body}");
        assert!(
            body["handle"].as_str().is_some_and(|h| !h.is_empty()),
            "{email}"
        );
        assert!(
            body["options"]
                .as_str()
                .is_some_and(|o| o.contains("challenge")),
            "{email}"
        );
    }
}

#[tokio::test]
async fn registering_needs_a_session() {
    let app = app().await;
    let (status, body, _) =
        post_json(&app, "/account/passkeys/begin", None, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn a_passkey_can_be_removed_without_any_script() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let mut authenticator = authenticator();
    register(&app, &mut authenticator, &token, "phone").await;

    // A plain form post — the delete path must work where the create
    // path cannot.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/account/passkeys/delete")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from({
                    let (_, page) = get(&app, "/account/passkeys", Some(&token)).await;
                    let at = page.find(r#"name="credential_id" value=""#).expect("an id");
                    let id: String = page[at + r#"name="credential_id" value=""#.len()..]
                        .chars()
                        .take_while(|c| *c != '"')
                        .collect();
                    format!("credential_id={id}")
                }))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let (_, page) = get(&app, "/account/passkeys", Some(&token)).await;
    assert!(page.contains("No passkeys yet"), "{page:.600}");
}
