//! Starting without an account and keeping the work, and attaching a
//! phone number to one.

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

/// Codes that would have gone to a phone.
#[derive(Clone, Default)]
struct Texts(Arc<Mutex<Vec<(String, String)>>>);

#[async_trait::async_trait]
impl auth_ui::mailer::SmsSender for Texts {
    async fn send_login_code(&self, to: &str, code: &str) {
        self.0
            .lock()
            .unwrap()
            .push((to.to_owned(), code.to_owned()));
    }
}

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: "http://localhost:8080".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    }
}

/// The app, plus whatever its phone page tried to text.
async fn app() -> (axum::Router, Texts) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).unwrap();
    let texts = Texts::default();
    let sms: Arc<dyn auth_ui::mailer::SmsSender> = Arc::new(texts.clone());
    let app = server::app_router_with_senders(
        &config,
        auth,
        std::sync::Arc::new(auth_server::oauth::SocialState::disabled()),
        None,
        Some(sms),
    );
    (app, texts)
}

struct Sent {
    status: StatusCode,
    location: String,
    cookies: Vec<String>,
    body: String,
}

async fn send(app: &axum::Router, request: Request<Body>) -> Sent {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_owned)
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Sent {
        status,
        location,
        cookies,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn post(app: &axum::Router, uri: &str, session: Option<&str>, form: &str) -> Sent {
    let mut builder = Request::builder().method("POST").uri(uri);
    if let Some(session) = session {
        builder = builder.header(header::COOKIE, format!("architect-auth.session={session}"));
    }
    send(
        app,
        builder
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_owned()))
            .unwrap(),
    )
    .await
}

async fn get(app: &axum::Router, uri: &str, session: Option<&str>) -> Sent {
    let mut builder = Request::builder().uri(uri);
    if let Some(session) = session {
        builder = builder.header(header::COOKIE, format!("architect-auth.session={session}"));
    }
    send(app, builder.body(Body::empty()).unwrap()).await
}

fn session_of(cookies: &[String]) -> String {
    cookies
        .iter()
        .find(|c| c.starts_with("architect-auth.session="))
        .and_then(|c| c.split_once('='))
        .map(|(_, rest)| rest.split(';').next().unwrap_or_default().to_owned())
        .unwrap_or_default()
}

// ── Guests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_guest_can_start_and_do_real_work() {
    let (app, _) = app().await;
    let started = post(&app, "/login/guest", None, "return_to=%2Forgs").await;
    assert_eq!(started.status, StatusCode::SEE_OTHER);
    assert_eq!(started.location, "/orgs");
    let guest = session_of(&started.cookies);
    assert!(!guest.is_empty());

    // A guest is a real account: it can own an organization.
    let made = post(&app, "/orgs", Some(&guest), "name=Trial+Run&slug=").await;
    assert_eq!(made.status, StatusCode::SEE_OTHER);
    assert!(made.location.starts_with("/orgs/"), "{}", made.location);
}

#[tokio::test]
async fn the_profile_page_warns_a_guest_first_and_plainly() {
    let (app, _) = app().await;
    let guest = session_of(&post(&app, "/login/guest", None, "").await.cookies);

    let page = get(&app, "/account/profile", Some(&guest)).await;
    assert_eq!(page.status, StatusCode::OK);
    // The rail says who you are; a guest has no address to show, so it
    // says what they have instead — nothing saved.
    assert!(page.body.contains("Not saved yet"), "{:.900}", page.body);
    assert!(
        page.body.contains("Save your account"),
        "{:.900}",
        page.body
    );
    // The warning has to say what is actually at stake, not "consider
    // creating an account".
    assert!(
        page.body.contains("everything you have made here is gone"),
        "{:.1200}",
        page.body
    );
    // Before the ordinary forms, which a guest cannot use yet.
    let warning_at = page.body.find("Save your account").unwrap();
    let details_at = page.body.find("Display name").unwrap();
    assert!(warning_at < details_at, "the warning must come first");
}

#[tokio::test]
async fn upgrading_keeps_the_same_account_and_everything_in_it() {
    let (app, _) = app().await;
    let guest = session_of(&post(&app, "/login/guest", None, "").await.cookies);
    let made = post(&app, "/orgs", Some(&guest), "name=Trial+Run&slug=trial").await;
    let org_path = made.location.clone();

    let upgraded = post(
        &app,
        "/account/upgrade",
        Some(&guest),
        "email=ada@example.com&password=correct+horse+battery+staple&name=Ada",
    )
    .await;
    assert_eq!(upgraded.status, StatusCode::SEE_OTHER);
    assert!(upgraded.location.contains("ok="), "{}", upgraded.location);

    // Upgrading issues a fresh session, so the cookie has to have been
    // replaced or the next click is signed out.
    let session = session_of(&upgraded.cookies);
    assert!(!session.is_empty(), "a new session cookie must be set");
    assert_ne!(session, guest, "the guest session is replaced");

    // Same account, same organization — nothing was copied anywhere.
    let org = get(&app, &org_path, Some(&session)).await;
    assert_eq!(org.status, StatusCode::OK);
    assert!(org.body.contains("Trial Run"), "{:.400}", org.body);

    // And it is no longer a guest.
    let profile = get(&app, "/account/profile", Some(&session)).await;
    assert!(
        !profile.body.contains("Not saved yet"),
        "{:.600}",
        profile.body
    );
    assert!(
        !profile.body.contains("Save your account"),
        "{:.600}",
        profile.body
    );
    assert!(profile.body.contains("ada@example.com"));

    // The new credentials work on their own.
    let signed_in = post(
        &app,
        "/login",
        None,
        "email=ada@example.com&password=correct+horse+battery+staple",
    )
    .await;
    assert_eq!(signed_in.status, StatusCode::SEE_OTHER);
    assert!(!session_of(&signed_in.cookies).is_empty());
}

#[tokio::test]
async fn the_sign_in_page_offers_the_guest_route() {
    let (app, _) = app().await;
    let page = get(&app, "/login", None).await;
    assert!(
        page.body.contains("Continue as a guest"),
        "{:.1200}",
        page.body
    );
}

// ── Phone numbers ────────────────────────────────────────────────────

#[tokio::test]
async fn a_number_is_only_saved_after_a_code_comes_back() {
    let (app, texts) = app().await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/sign-up-email-password")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"input":{"email":"ada@example.com","password":"correct horse battery staple"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let session = json["token"].as_str().unwrap().to_owned();

    let page = get(&app, "/account/phone", Some(&session)).await;
    assert!(
        page.body.contains("No phone number on this account"),
        "{:.400}",
        page.body
    );

    let sent = post(
        &app,
        "/account/phone",
        Some(&session),
        "phone_number=%2B447700900000",
    )
    .await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER);
    assert!(sent.location.contains("sent=1"), "{}", sent.location);

    let (to, code) = texts.0.lock().unwrap().last().cloned().expect("a text");
    assert!(to.contains("447700900000"), "{to}");

    // The number is claimed straight away and marked unconfirmed. It
    // has to be: `verify_phone_number_otp` attaches the number to
    // whoever already holds it and mints a NEW account when nobody
    // does, so verifying before claiming would create a stray user and
    // then refuse to give this account the number. What the page must
    // not do is imply the number works before it has been tested.
    let before = get(&app, "/account/phone", Some(&session)).await;
    assert!(
        before.body.contains("not confirmed yet"),
        "{:.500}",
        before.body
    );

    let wrong = post(
        &app,
        "/account/phone/verify",
        Some(&session),
        "phone_number=%2B447700900000&code=000000",
    )
    .await;
    assert!(wrong.location.contains("error="), "{}", wrong.location);

    let right = post(
        &app,
        "/account/phone/verify",
        Some(&session),
        &format!("phone_number=%2B447700900000&code={code}"),
    )
    .await;
    assert!(right.location.contains("ok="), "{}", right.location);

    let after = get(&app, "/account/phone", Some(&session)).await;
    assert!(after.body.contains("confirmed"), "{:.500}", after.body);
    assert!(
        !after.body.contains("not confirmed yet"),
        "{:.500}",
        after.body
    );
    assert!(after.body.contains("447700900000"), "{:.400}", after.body);
}

#[tokio::test]
async fn the_phone_page_needs_a_session() {
    // Not a sign-in route: an unauthenticated endpoint that texts any
    // number typed into it is a bill somebody else pays.
    let (app, texts) = app().await;
    let page = get(&app, "/account/phone", None).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert!(
        page.location.starts_with("/login?return_to="),
        "{}",
        page.location
    );

    let sent = post(&app, "/account/phone", None, "phone_number=%2B447700900000").await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER);
    assert!(sent.location.starts_with("/login?"), "{}", sent.location);
    assert!(texts.0.lock().unwrap().is_empty(), "nothing may be sent");
}

// ── Wallets ──────────────────────────────────────────────────────────

/// Sign the way a wallet's `personal_sign` does.
fn wallet_sign(key: &k256::ecdsa::SigningKey, message: &str) -> String {
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use sha3::{Digest as _, Keccak256};

    let mut prefixed = format!("\x19Ethereum Signed Message:\n{}", message.len()).into_bytes();
    prefixed.extend_from_slice(message.as_bytes());
    let digest: [u8; 32] = Keccak256::digest(&prefixed).into();
    let (signature, recovery): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) =
        key.sign_prehash(&digest).unwrap();
    let mut bytes = signature.to_bytes().to_vec();
    bytes.push(recovery.to_byte() + 27);
    format!("0x{}", hex::encode(bytes))
}

async fn post_json(
    app: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value, Vec<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_owned)
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        cookies,
    )
}

#[tokio::test]
async fn a_wallet_signature_signs_you_in_over_http() {
    let (app, _) = app().await;
    let key = k256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();

    let (status, start, _) = post_json(&app, "/login/wallet/begin", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "{start}");
    let template = start["message"].as_str().expect("a message").to_owned();
    // The server composes it, and its first line is the domain — which
    // is what stops a signature collected elsewhere being replayed here.
    assert!(template.starts_with("localhost"), "{template}");
    assert!(template.contains("Nonce: "), "{template}");

    // The browser fills the address in; the signature is checked
    // against whatever ends up there.
    let address = {
        use sha3::{Digest as _, Keccak256};
        let encoded = key.verifying_key().to_sec1_point(false);
        let hashed = Keccak256::digest(&encoded.as_bytes()[1..]);
        format!("0x{}", hex::encode(&hashed[12..]))
    };
    let message = template.replace("\nURI:", &format!("\nAddress: {address}\nURI:"));
    let signature = wallet_sign(&key, &message);

    let (status, done, cookies) = post_json(
        &app,
        "/login/wallet/complete",
        serde_json::json!({ "message": message, "signature": signature, "return_to": "/orgs" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["redirect"], "/orgs");
    let session = session_of(&cookies);
    assert!(!session.is_empty());

    let orgs = get(&app, "/orgs", Some(&session)).await;
    assert_eq!(orgs.status, StatusCode::OK);
}

#[tokio::test]
async fn a_signature_from_the_wrong_key_is_refused_over_http() {
    let (app, _) = app().await;
    let mallory = k256::ecdsa::SigningKey::from_slice(&[3u8; 32]).unwrap();
    let victim = {
        use sha3::{Digest as _, Keccak256};
        let key = k256::ecdsa::SigningKey::from_slice(&[5u8; 32]).unwrap();
        let encoded = key.verifying_key().to_sec1_point(false);
        let hashed = Keccak256::digest(&encoded.as_bytes()[1..]);
        format!("0x{}", hex::encode(&hashed[12..]))
    };

    let (_, start, _) = post_json(&app, "/login/wallet/begin", serde_json::json!({})).await;
    let template = start["message"].as_str().unwrap().to_owned();
    // The victim's address in the text, Mallory's key on the signature.
    let message = template.replace("\nURI:", &format!("\nAddress: {victim}\nURI:"));
    let signature = wallet_sign(&mallory, &message);

    let (status, body, cookies) = post_json(
        &app,
        "/login/wallet/complete",
        serde_json::json!({ "message": message, "signature": signature }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(session_of(&cookies).is_empty(), "no session may be issued");
}
