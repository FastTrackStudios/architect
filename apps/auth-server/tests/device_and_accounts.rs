//! Connecting a device with no browser, and holding several accounts
//! in one that has.

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

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        base_url: "http://localhost:8080".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    }
}

async fn app() -> (
    axum::Router,
    architect_auth::ArchitectAuth<AuthSeaOrmStorage>,
) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).unwrap();
    let app = server::app_router(&config, auth.clone()).unwrap();
    (app, auth)
}

async fn signed_up(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/sign-up-email-password")
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

/// A request carrying a session cookie and, optionally, a roster.
fn request(method: &str, uri: &str, session: Option<&str>, roster: Option<&str>) -> Request<Body> {
    let mut cookies = Vec::new();
    if let Some(session) = session {
        cookies.push(format!("architect-auth.session={session}"));
    }
    if let Some(roster) = roster {
        cookies.push(format!("architect-auth.session.accounts={roster}"));
    }
    let mut builder = Request::builder().method(method).uri(uri);
    if !cookies.is_empty() {
        builder = builder.header(header::COOKIE, cookies.join("; "));
    }
    builder
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::empty())
        .unwrap()
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

async fn post_form(
    app: &axum::Router,
    uri: &str,
    session: Option<&str>,
    roster: Option<&str>,
    form: &str,
) -> Sent {
    let mut cookies = Vec::new();
    if let Some(session) = session {
        cookies.push(format!("architect-auth.session={session}"));
    }
    if let Some(roster) = roster {
        cookies.push(format!("architect-auth.session.accounts={roster}"));
    }
    let mut builder = Request::builder().method("POST").uri(uri);
    if !cookies.is_empty() {
        builder = builder.header(header::COOKIE, cookies.join("; "));
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

fn roster_of(cookies: &[String]) -> Option<String> {
    cookies
        .iter()
        .find(|c| c.starts_with("architect-auth.session.accounts="))
        .and_then(|c| c.split_once('='))
        .map(|(_, rest)| rest.split(';').next().unwrap_or_default().to_owned())
}

// ── A device with no browser ─────────────────────────────────────────

#[tokio::test]
async fn a_device_code_is_shown_before_it_is_approved() {
    let (app, auth) = app().await;
    let token = signed_up(&app, "ada@example.com").await;

    let device = auth
        .create_device_authorization(architect_auth::CreateDeviceAuthorization {
            client_id: "living-room-tv".into(),
            scope: Some("openid profile".into()),
            expires_in_seconds: None,
            interval_seconds: None,
        })
        .await
        .expect("device authorization");

    // The URI the device prints must be one this server serves.
    assert_eq!(device.verification_uri, "/auth/device");

    let page = send(
        &app,
        request(
            "GET",
            &format!("/auth/device?user_code={}", device.user_code),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    // Who is asking, and for what — before anybody agrees to it.
    assert!(page.body.contains("living-room-tv"), "{:.500}", page.body);
    assert!(page.body.contains("openid"), "{:.700}", page.body);
    assert!(page.body.contains("profile"), "{:.700}", page.body);
    assert!(
        page.body
            .contains("Nobody should ever ask you for this code")
    );
}

#[tokio::test]
async fn a_code_typed_with_dashes_and_lowercase_still_works() {
    let (app, auth) = app().await;
    let token = signed_up(&app, "ada@example.com").await;
    let device = auth
        .create_device_authorization(architect_auth::CreateDeviceAuthorization {
            client_id: "cli".into(),
            scope: None,
            expires_in_seconds: None,
            interval_seconds: None,
        })
        .await
        .unwrap();

    // As read off a screen across the room.
    let typed = format!(
        "{}-{}",
        &device.user_code[..device.user_code.len() / 2].to_lowercase(),
        &device.user_code[device.user_code.len() / 2..].to_lowercase()
    );
    let looked_up = post_form(
        &app,
        "/auth/device",
        Some(&token),
        None,
        &format!("user_code={typed}"),
    )
    .await;
    assert_eq!(looked_up.status, StatusCode::SEE_OTHER);

    let approved = post_form(
        &app,
        "/auth/device/approve",
        Some(&token),
        None,
        &format!("user_code={typed}"),
    )
    .await;
    assert_eq!(approved.status, StatusCode::OK);
    assert!(
        approved.body.contains("Device connected"),
        "{:.400}",
        approved.body
    );

    // And the device, polling, now gets a session.
    let polled = auth
        .poll_device_token(architect_auth::PollDeviceToken {
            device_code: device.device_code,
            ip_address: None,
            user_agent: None,
        })
        .await
        .expect("the device is signed in");
    assert_eq!(polled.user.email.as_deref(), Some("ada@example.com"));
}

#[tokio::test]
async fn refusing_a_code_needs_no_session_at_all() {
    let (app, auth) = app().await;
    let device = auth
        .create_device_authorization(architect_auth::CreateDeviceAuthorization {
            client_id: "cli".into(),
            scope: None,
            expires_in_seconds: None,
            interval_seconds: None,
        })
        .await
        .unwrap();

    // Somebody who was read a code over the phone and thought better of
    // it must be able to shut it down without first proving who they are.
    let denied = post_form(
        &app,
        "/auth/device/deny",
        None,
        None,
        &format!("user_code={}", device.user_code),
    )
    .await;
    assert_eq!(denied.status, StatusCode::OK);
    assert!(
        denied.body.contains("Device refused"),
        "{:.400}",
        denied.body
    );

    let polled = auth
        .poll_device_token(architect_auth::PollDeviceToken {
            device_code: device.device_code,
            ip_address: None,
            user_agent: None,
        })
        .await;
    assert!(polled.is_err(), "a refused code must not sign anything in");
}

#[tokio::test]
async fn the_device_page_sends_a_signed_out_visitor_to_sign_in_and_back() {
    let (app, _) = app().await;
    let page = send(
        &app,
        request("GET", "/auth/device?user_code=WDJB-MJHT", None, None),
    )
    .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert!(
        page.location.starts_with("/login?return_to="),
        "{}",
        page.location
    );
    // The code has to survive the round trip, or they have to go and
    // read it off the television again.
    assert!(page.location.contains("user_code"), "{}", page.location);
}

// ── Several accounts in one browser ──────────────────────────────────

#[tokio::test]
async fn signing_in_twice_puts_both_accounts_on_the_switcher() {
    let (app, _) = app().await;
    signed_up(&app, "ada@example.com").await;
    signed_up(&app, "grace@example.com").await;

    let first = post_form(
        &app,
        "/login",
        None,
        None,
        "email=ada@example.com&password=correct+horse+battery+staple",
    )
    .await;
    let ada_roster = roster_of(&first.cookies).expect("a roster");
    let ada_token = ada_roster.clone();

    // Signing in as somebody else, with the first roster still held.
    let second = post_form(
        &app,
        "/login",
        None,
        Some(&ada_roster),
        "email=grace@example.com&password=correct+horse+battery+staple",
    )
    .await;
    let both = roster_of(&second.cookies).expect("a roster");
    assert_eq!(both.split(',').count(), 2, "{both}");
    assert!(
        both.contains(&ada_token),
        "the first account must survive: {both}"
    );

    let grace_token = both.split(',').next().unwrap().to_owned();
    let page = send(
        &app,
        request("GET", "/account/switch", Some(&grace_token), Some(&both)),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("ada@example.com"), "{:.900}", page.body);
    assert!(
        page.body.contains("grace@example.com"),
        "{:.900}",
        page.body
    );
    assert!(page.body.contains("Current"));
}

#[tokio::test]
async fn switching_promotes_a_held_token_without_signing_anybody_out() {
    let (app, _) = app().await;
    signed_up(&app, "ada@example.com").await;
    signed_up(&app, "grace@example.com").await;
    let ada = roster_of(
        &post_form(
            &app,
            "/login",
            None,
            None,
            "email=ada@example.com&password=correct+horse+battery+staple",
        )
        .await
        .cookies,
    )
    .unwrap();
    let both = roster_of(
        &post_form(
            &app,
            "/login",
            None,
            Some(&ada),
            "email=grace@example.com&password=correct+horse+battery+staple",
        )
        .await
        .cookies,
    )
    .unwrap();
    let grace = both.split(',').next().unwrap().to_owned();

    let switched = post_form(
        &app,
        "/account/switch",
        Some(&grace),
        Some(&both),
        &format!("token={ada}"),
    )
    .await;
    assert_eq!(switched.status, StatusCode::SEE_OTHER);
    // The session cookie is now Ada's...
    assert!(
        switched
            .cookies
            .iter()
            .any(|c| c.starts_with(&format!("architect-auth.session={ada}"))),
        "{:?}",
        switched.cookies
    );
    // ...and Grace is still signed in, still on the roster.
    let roster = roster_of(&switched.cookies).unwrap();
    assert!(roster.contains(&grace), "{roster}");
}

#[tokio::test]
async fn a_token_this_browser_does_not_hold_cannot_be_switched_to() {
    // Without this the form is "paste a session token here and become
    // that person".
    let (app, _) = app().await;
    signed_up(&app, "ada@example.com").await;
    let victim = signed_up(&app, "grace@example.com").await;
    let ada = roster_of(
        &post_form(
            &app,
            "/login",
            None,
            None,
            "email=ada@example.com&password=correct+horse+battery+staple",
        )
        .await
        .cookies,
    )
    .unwrap();

    let stolen = post_form(
        &app,
        "/account/switch",
        Some(&ada),
        Some(&ada),
        &format!("token={victim}"),
    )
    .await;
    assert_eq!(stolen.status, StatusCode::SEE_OTHER);
    assert!(stolen.location.contains("error="), "{}", stolen.location);
    assert!(
        !stolen
            .cookies
            .iter()
            .any(|c| c.starts_with(&format!("architect-auth.session={victim}"))),
        "a token this browser never held must not become the session"
    );
}

#[tokio::test]
async fn leaving_one_account_promotes_another_rather_than_signing_you_out() {
    let (app, _) = app().await;
    signed_up(&app, "ada@example.com").await;
    signed_up(&app, "grace@example.com").await;
    let ada = roster_of(
        &post_form(
            &app,
            "/login",
            None,
            None,
            "email=ada@example.com&password=correct+horse+battery+staple",
        )
        .await
        .cookies,
    )
    .unwrap();
    let both = roster_of(
        &post_form(
            &app,
            "/login",
            None,
            Some(&ada),
            "email=grace@example.com&password=correct+horse+battery+staple",
        )
        .await
        .cookies,
    )
    .unwrap();
    let grace = both.split(',').next().unwrap().to_owned();

    let left = post_form(
        &app,
        "/account/switch/leave",
        Some(&grace),
        Some(&both),
        &format!("token={grace}"),
    )
    .await;
    assert_eq!(left.status, StatusCode::SEE_OTHER);
    assert_eq!(left.location, "/account/switch", "{}", left.location);
    // Ada is now the session, and the roster holds only her.
    assert!(
        left.cookies
            .iter()
            .any(|c| c.starts_with(&format!("architect-auth.session={ada}"))),
        "{:?}",
        left.cookies
    );
    assert_eq!(roster_of(&left.cookies).as_deref(), Some(ada.as_str()));
}

#[tokio::test]
async fn leaving_the_last_account_goes_to_sign_in() {
    let (app, _) = app().await;
    signed_up(&app, "ada@example.com").await;
    let ada = roster_of(
        &post_form(
            &app,
            "/login",
            None,
            None,
            "email=ada@example.com&password=correct+horse+battery+staple",
        )
        .await
        .cookies,
    )
    .unwrap();

    let left = post_form(
        &app,
        "/account/switch/leave",
        Some(&ada),
        Some(&ada),
        &format!("token={ada}"),
    )
    .await;
    assert_eq!(left.location, "/login");
    assert!(
        roster_of(&left.cookies).is_some_and(|r| r.is_empty()),
        "{:?}",
        left.cookies
    );
}
