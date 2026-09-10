//! Signing in without a password, and signing in by username.
//!
//! The mailer is captured rather than mocked away: these tests read the
//! code and the link out of what would have been sent and then use
//! them, so the whole path is exercised — including the URL the server
//! builds, which is the part most likely to be quietly wrong.

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

use std::sync::{Arc, Mutex};

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

const BASE: &str = "http://localhost:8080";

/// Everything the pages tried to send.
#[derive(Clone, Default)]
struct Outbox(Arc<Mutex<Vec<(String, String, String)>>>);

impl Outbox {
    fn record(&self, kind: &str, to: &str, body: &str) {
        self.0
            .lock()
            .unwrap()
            .push((kind.to_owned(), to.to_owned(), body.to_owned()));
    }

    /// The most recent thing sent to this address of this kind.
    fn latest(&self, kind: &str, to: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(k, t, _)| k == kind && t == to)
            .map(|(_, _, body)| body.clone())
    }

    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl auth_ui::mailer::LoginMailer for Outbox {
    async fn send_magic_link(&self, to: &str, url: &str) {
        self.record("link", to, url);
    }
    async fn send_login_code(&self, to: &str, code: &str) {
        self.record("code", to, code);
    }
}

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: BASE.into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    }
}

/// The app, plus the outbox its sign-in pages write to.
async fn app() -> (axum::Router, Outbox) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db.clone())).unwrap();
    let outbox = Outbox::default();
    let ui: Arc<dyn auth_ui::mailer::LoginMailer> = Arc::new(outbox.clone());
    let app = server::app_router_with_mailer(
        &config,
        auth,
        std::sync::Arc::new(auth_server::http::SocialState::disabled()),
        Some(ui),
    );
    (app, outbox)
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
                    r#"{{"email":"{email}","password":"correct horse battery staple"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["token"].as_str().unwrap().to_owned()
}

async fn get(app: &axum::Router, uri: &str, token: Option<&str>) -> (StatusCode, String, String) {
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
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let _ = String::from_utf8_lossy(&bytes);
    (status, location, set_cookie)
}

async fn body_of(app: &axum::Router, uri: &str) -> String {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn post(app: &axum::Router, uri: &str, form: &str) -> (StatusCode, String, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (status, location, set_cookie)
}

fn cookie_value(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .and_then(|c| c.split_once('='))
        .map(|(_, v)| v.to_owned())
        .unwrap_or_default()
}

// ── A code ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_mailed_code_signs_you_in() {
    let (app, outbox) = app().await;
    signed_up(&app, "ada@example.com").await;

    let (status, location, _) = post(
        &app,
        "/login/code",
        "email=ada@example.com&return_to=%2Forgs",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.starts_with("/login/code?sent=1"), "{location}");

    let code = outbox.latest("code", "ada@example.com").expect("a code");
    assert_eq!(
        code.chars().count(),
        6,
        "short enough to read aloud: {code}"
    );
    // Uppercase base64url, not digits. Worth pinning: the entry field
    // asked for a numeric keypad until a test printed a real one, and a
    // numeric keypad cannot type `C2J-ZN`.
    assert!(
        code.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-' || c == '_'),
        "{code}"
    );

    let (status, location, set_cookie) = post(
        &app,
        "/login/code/verify",
        &format!("email=ada@example.com&code={code}&return_to=%2Forgs"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/orgs");

    // And the cookie is a working session.
    let (status, _, _) = get(&app, "/orgs", Some(&cookie_value(&set_cookie))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_code_works_once() {
    let (app, outbox) = app().await;
    signed_up(&app, "ada@example.com").await;
    post(&app, "/login/code", "email=ada@example.com").await;
    let code = outbox.latest("code", "ada@example.com").unwrap();

    post(
        &app,
        "/login/code/verify",
        &format!("email=ada@example.com&code={code}"),
    )
    .await;
    let (_, location, _) = post(
        &app,
        "/login/code/verify",
        &format!("email=ada@example.com&code={code}"),
    )
    .await;
    assert!(
        location.contains("error="),
        "a spent code must not work again"
    );
}

#[tokio::test]
async fn a_wrong_code_says_so_without_saying_whose() {
    let (app, _) = app().await;
    signed_up(&app, "ada@example.com").await;
    let (_, location, set_cookie) = post(
        &app,
        "/login/code/verify",
        "email=ada@example.com&code=000000",
    )
    .await;
    assert!(location.contains("error="), "{location}");
    assert!(set_cookie.is_empty(), "no session on a bad code");
}

// ── A link ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_mailed_link_signs_you_in() {
    let (app, outbox) = app().await;
    signed_up(&app, "ada@example.com").await;

    let (status, location, _) = post(
        &app,
        "/login/link",
        "email=ada@example.com&return_to=%2Forgs",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.starts_with("/login/link?sent=1"), "{location}");

    let url = outbox.latest("link", "ada@example.com").expect("a link");
    // The callback must be a path this router actually serves — an
    // engine default would have pointed at the site root.
    assert!(
        url.starts_with(&format!("{BASE}/login/magic?")),
        "the link must land on the callback: {url}"
    );
    assert!(url.contains("return_to="), "{url}");

    // Open it, as a browser would.
    let path = url.strip_prefix(BASE).unwrap();
    let (status, location, set_cookie) = get(&app, path, None).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{location}");
    assert_eq!(location, "/orgs");
    let (status, _, _) = get(&app, "/orgs", Some(&cookie_value(&set_cookie))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_link_works_once_and_then_offers_a_new_one() {
    let (app, outbox) = app().await;
    signed_up(&app, "ada@example.com").await;
    post(&app, "/login/link", "email=ada@example.com").await;
    let url = outbox.latest("link", "ada@example.com").unwrap();
    let path = url.strip_prefix(BASE).unwrap().to_owned();

    let (status, _, _) = get(&app, &path, None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    // A mail scanner that follows links before the person does is a
    // real thing, so the second open has to explain itself rather than
    // just failing.
    let page = body_of(&app, &path).await;
    assert!(page.contains("not usable"), "{page:.400}");
    assert!(page.contains("Send a new link"), "{page:.600}");
}

#[tokio::test]
async fn a_tampered_link_is_refused() {
    let (app, outbox) = app().await;
    signed_up(&app, "ada@example.com").await;
    post(&app, "/login/link", "email=ada@example.com").await;
    let url = outbox.latest("link", "ada@example.com").unwrap();
    let path = url.strip_prefix(BASE).unwrap();

    // Same token, somebody else's address.
    let swapped = path.replace("ada%40example.com", "mallory%40example.com");
    let (status, _, set_cookie) = get(&app, &swapped, None).await;
    assert_eq!(status, StatusCode::OK, "an error page, not a redirect");
    assert!(set_cookie.is_empty(), "no session may be handed out");
}

// ── The oracle ───────────────────────────────────────────────────────

#[tokio::test]
async fn asking_about_a_stranger_looks_exactly_like_asking_about_a_member() {
    let (app, outbox) = app().await;
    signed_up(&app, "ada@example.com").await;

    // A registered address, an unknown one, and a malformed one: same
    // status, same destination. Anything else answers "does this person
    // have an account here?" to whoever asks.
    let mut seen = Vec::new();
    for (path, email) in [
        ("/login/code", "ada@example.com"),
        ("/login/code", "nobody@example.com"),
        ("/login/code", "not-an-address"),
        ("/login/link", "ada@example.com"),
        ("/login/link", "nobody@example.com"),
        ("/login/link", "not-an-address"),
    ] {
        let (status, location, _) = post(&app, path, &format!("email={email}")).await;
        seen.push((
            status,
            location.split('&').next().unwrap_or_default().to_owned(),
        ));
    }
    assert_eq!(seen[0], seen[1], "known vs unknown, code");
    assert_eq!(seen[1], seen[2], "unknown vs malformed, code");
    assert_eq!(seen[3], seen[4], "known vs unknown, link");
    assert_eq!(seen[4], seen[5], "unknown vs malformed, link");

    // Four sends, not two: an address with no account still gets a
    // code or a link, because using one *creates* the account —
    // `verify_magic_link` signs you up. That is the passwordless
    // onboarding model, and it is also what makes the answers above
    // indistinguishable without any special-casing. Only the malformed
    // address is dropped, before anything is minted.
    assert_eq!(outbox.count(), 4);
}

// ── Username ─────────────────────────────────────────────────────────

#[tokio::test]
async fn one_field_takes_an_address_or_a_username() {
    let (app, _) = app().await;
    let token = signed_up(&app, "ada@example.com").await;

    // Claim a username through the profile page.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/account/profile")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("name=Ada&username=ada&image="))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // The same form and the same field, with a username in it.
    let (status, location, set_cookie) = post(
        &app,
        "/login",
        "email=ada&password=correct+horse+battery+staple&return_to=%2Forgs",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/orgs", "a username must sign in");
    assert!(!set_cookie.is_empty());

    // And the address still works.
    let (_, location, _) = post(
        &app,
        "/login",
        "email=ada@example.com&password=correct+horse+battery+staple&return_to=%2Forgs",
    )
    .await;
    assert_eq!(location, "/orgs");
}

#[tokio::test]
async fn a_wrong_username_is_refused_like_a_wrong_address() {
    let (app, _) = app().await;
    signed_up(&app, "ada@example.com").await;
    // Neither of these should distinguish "no such account" from
    // "wrong password" — they render the same screen.
    for form in [
        "email=nosuchuser&password=whatever",
        "email=nobody@example.com&password=whatever",
    ] {
        let (status, _, set_cookie) = post(&app, "/login", form).await;
        assert_eq!(status, StatusCode::OK, "{form}");
        assert!(set_cookie.is_empty(), "{form}");
    }
}

#[tokio::test]
async fn the_sign_in_field_accepts_a_username_in_the_markup_too() {
    let (app, _) = app().await;
    let page = body_of(&app, "/login").await;
    // `type="email"` would have the browser refuse a username before
    // the form was ever submitted.
    assert!(page.contains("Email or username"), "{page:.900}");
    assert!(
        !page.contains(r#"id="email" name="email" type="email""#),
        "{page:.900}"
    );
    assert!(
        page.contains("/login/link"),
        "the alternatives must be offered"
    );
    assert!(page.contains("/login/code"));
}

#[tokio::test]
async fn a_code_typed_in_lowercase_still_works() {
    // Codes are minted uppercase and a phone keyboard will not
    // capitalise, so this is what most people will actually type.
    let (app, outbox) = app().await;
    signed_up(&app, "ada@example.com").await;
    post(&app, "/login/code", "email=ada@example.com").await;
    let code = outbox.latest("code", "ada@example.com").unwrap();

    let (status, location, set_cookie) = post(
        &app,
        "/login/code/verify",
        &format!("email=ada@example.com&code={}", code.to_lowercase()),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(!location.contains("error="), "{location}");
    assert!(!set_cookie.is_empty(), "a lowercase code must sign you in");
}

#[tokio::test]
async fn a_link_signs_up_somebody_who_had_no_account() {
    // The other half of the model above: the link is the account.
    let (app, outbox) = app().await;
    post(&app, "/login/link", "email=newcomer@example.com").await;
    let url = outbox
        .latest("link", "newcomer@example.com")
        .expect("a link");
    let path = url.strip_prefix(BASE).unwrap();

    let (status, location, set_cookie) = get(&app, path, None).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{location}");
    let (status, _, _) = get(&app, "/orgs", Some(&cookie_value(&set_cookie))).await;
    assert_eq!(status, StatusCode::OK, "the new account is signed in");
}
