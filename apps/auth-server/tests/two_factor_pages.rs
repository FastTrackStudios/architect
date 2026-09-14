//! Signing in with two-factor on, through the pages.
//!
//! The reason this file exists: before it, a person with two-factor
//! enabled could not use the web UI at all. Sign-in issued a session
//! that is deliberately inactive until the second factor is given,
//! every page treats an inactive session as no session, and the login
//! form sent the browser to the account page regardless — which
//! bounced to /login, which signed in again, forever. Nothing failed
//! loudly; the browser just spun.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::panic
)]

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::{ServerConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::Database;
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
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
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (status, location, set_cookie)
}

/// The `value="…"` of the input carrying `aria-label="<label>"`.
///
/// Searches backwards from the label rather than assuming an attribute
/// order: the renderer is free to emit them in any order, and it does
/// not put `value` where a naive forward scan would look.
fn value_of_input_labelled(html: &str, label: &str) -> String {
    let marker = format!(r#"aria-label="{label}""#);
    let at = html.find(&marker).expect("an input with that label");
    let before = html.get(..at).expect("text before the label");
    let value_at = before.rfind(r#"value=""#).expect("a value attribute");
    before
        .get(value_at + r#"value=""#.len()..)
        .expect("the value")
        .split('"')
        .next()
        .expect("its end")
        .to_owned()
}

/// The code an authenticator app would show for this secret right now.
fn totp_code(secret: &str) -> String {
    use totp_rs::{Algorithm, Secret, TOTP};
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Encoded(secret.to_owned()).to_bytes().unwrap(),
        None,
        "architect-auth".into(),
    )
    .unwrap()
    .generate_current()
    .unwrap()
}

/// Enrol and confirm, returning the secret and the backup codes.
async fn enrolled(app: &axum::Router, token: &str) -> (String, Vec<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/account/two-factor/enroll")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let page = String::from_utf8_lossy(&bytes).into_owned();

    let secret = value_of_input_labelled(&page, "Setup key");
    let codes: Vec<String> = page
        .split(r#"<li class="mono">"#)
        .skip(1)
        .filter_map(|chunk| chunk.split('<').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(codes.len(), 10, "backup codes on the page: {page:.900}");

    let (status, location, _) = post(
        app,
        "/account/two-factor/confirm",
        Some(token),
        &format!("code={}", totp_code(&secret)),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.contains("ok="), "{location}");
    (secret, codes)
}

#[tokio::test]
async fn the_enrolment_page_shows_a_qr_a_secret_and_ten_codes() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let (secret, codes) = enrolled(&app, &token).await;

    assert!(!secret.is_empty());
    assert_eq!(codes.len(), 10);
    // Typeable without thinking: the stored hash is over exactly the
    // string shown, so a separator here becomes a code that is right
    // and gets rejected.
    for code in &codes {
        assert!(
            code.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "{code:?}"
        );
    }

    let (_, _, page) = get(&app, "/account/two-factor", Some(&token)).await;
    assert!(page.contains("Signing in asks for a code"), "{page:.500}");
}

#[tokio::test]
async fn a_second_factor_account_can_still_sign_in_through_the_form() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let (secret, _) = enrolled(&app, &token).await;

    // The form post, exactly as the login page makes it.
    let (status, location, set_cookie) = post(
        &app,
        "/login",
        None,
        "email=ada@example.com&password=correct+horse+battery+staple&return_to=%2Forgs",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    // THE regression: this used to be `/orgs`, which bounced to /login,
    // which signed in again, forever.
    assert!(
        location.starts_with("/login/two-factor"),
        "a pending sign-in must go to the challenge, got {location}"
    );
    assert!(location.contains("return_to="), "{location}");
    // The cookie is still set — the challenge needs it to know whose
    // session to activate.
    assert!(set_cookie.contains('='), "{set_cookie}");

    let pending = set_cookie
        .split(';')
        .next()
        .and_then(|c| c.split_once('='))
        .map(|(_, v)| v.to_owned())
        .expect("a session cookie");

    // The challenge page renders for a session no other page accepts.
    let (status, _, page) = get(&app, "/login/two-factor", Some(&pending)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("One more step"), "{page:.400}");

    // A wrong code comes back to the challenge, not to a dead end.
    let (status, location, _) = post(
        &app,
        "/login/two-factor",
        Some(&pending),
        "code=000000&return_to=%2Forgs",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.starts_with("/login/two-factor"), "{location}");
    assert!(location.contains("error="), "{location}");

    // The right code activates the session and honours return_to.
    let (status, location, _) = post(
        &app,
        "/login/two-factor",
        Some(&pending),
        &format!("code={}&return_to=%2Forgs", totp_code(&secret)),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/orgs");

    // And that same token now works everywhere.
    let (status, _, page) = get(&app, "/orgs", Some(&pending)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Organizations"), "{page:.300}");
}

#[tokio::test]
async fn a_backup_code_gets_you_in_when_the_phone_is_gone() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let (_, codes) = enrolled(&app, &token).await;

    let (_, _, set_cookie) = post(
        &app,
        "/login",
        None,
        "email=ada@example.com&password=correct+horse+battery+staple",
    )
    .await;
    let pending = set_cookie
        .split(';')
        .next()
        .and_then(|c| c.split_once('='))
        .map(|(_, v)| v.to_owned())
        .unwrap();

    let (status, location, _) = post(
        &app,
        "/login/two-factor",
        Some(&pending),
        &format!("code={}", codes[0]),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/");

    // Single use: the same code must not work twice.
    let (_, _, set_cookie) = post(
        &app,
        "/login",
        None,
        "email=ada@example.com&password=correct+horse+battery+staple",
    )
    .await;
    let second = set_cookie
        .split(';')
        .next()
        .and_then(|c| c.split_once('='))
        .map(|(_, v)| v.to_owned())
        .unwrap();
    let (_, location, _) = post(
        &app,
        "/login/two-factor",
        Some(&second),
        &format!("code={}", codes[0]),
    )
    .await;
    assert!(
        location.contains("error="),
        "a spent code must not work again"
    );
}

#[tokio::test]
async fn the_challenge_refuses_a_browser_with_no_session_at_all() {
    let app = app().await;
    let (status, location, _) = get(&app, "/login/two-factor", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/login");
}

#[tokio::test]
async fn turning_it_off_needs_proof_and_then_sign_in_is_one_step_again() {
    let app = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let (secret, _) = enrolled(&app, &token).await;

    let (_, location, _) = post(
        &app,
        "/account/two-factor/disable",
        Some(&token),
        "code=000000",
    )
    .await;
    assert!(
        location.contains("error="),
        "a wrong code must not disable it"
    );

    let (_, location, _) = post(
        &app,
        "/account/two-factor/disable",
        Some(&token),
        &format!("code={}", totp_code(&secret)),
    )
    .await;
    assert!(location.contains("ok="), "{location}");

    // Back to one step.
    let (status, location, _) = post(
        &app,
        "/login",
        None,
        "email=ada@example.com&password=correct+horse+battery+staple&return_to=%2Forgs",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/orgs");
}
