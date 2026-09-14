//! The real provider client against the real mock, over a real socket.
//!
//! Every other social test swaps the network half for a fake, which
//! proves the routes behave but says nothing about whether
//! `HttpProviderClient` can actually parse what a provider sends. This
//! one runs `mock-oauth` on a loopback port and drives the whole
//! authorization-code flow through it: the redirect out, the account
//! picker, the code, the token exchange, and the profile — including
//! GitHub's second call for the address, which is the step the mock
//! exists to exercise.

// See the note in `social.rs`: `allow-*-in-tests` does not reach a
// `tests/` file's helper code.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::sync::Arc;

use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::oauth::SocialState;
use auth_server::social::HttpProviderClient;
use auth_server::{ServerConfig, SocialProviderConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

const BASE: &str = "http://localhost:8080";

/// Start the mock on an arbitrary port and return its origin.
async fn mock_origin() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, mock_oauth::router()).await.ok();
    });
    format!("http://{addr}")
}

/// An auth server whose providers all point at `mock`.
async fn app(mock: &str) -> axum::Router {
    let db = Database::connect("sqlite::memory:").await.expect("connect");
    Migrator::up(&db, None).await.expect("migrate");
    let mut config = ServerConfig {
        base_url: BASE.into(),
        ..ServerConfig::local()
    };
    for slot in [
        &mut config.social.github,
        &mut config.social.google,
        &mut config.social.tone3000,
    ] {
        *slot = Some(SocialProviderConfig {
            client_id: "mock-client-id".into(),
            client_secret: "mock-client-secret".into(),
            scopes: vec!["email".into()],
        });
    }
    config.social.mock_url = Some(mock.to_owned());
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("engine");
    let social = Arc::new(SocialState {
        config: config.social.clone(),
        client: Arc::new(
            HttpProviderClient::new()
                .expect("http client")
                .with_mock(Some(mock.to_owned())),
        ),
        base_url: config.base_url.clone(),
        allowed_return_origins: Vec::new(),
    });
    server::app_router_with_social(&config, auth, social)
}

fn location(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("location")
        .to_str()
        .expect("ascii location")
        .to_owned()
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.replace("%2F", "/").replace("%3A", ":"))
    })
}

/// Walk the mock's account picker the way a person would: fetch the
/// authorize page, pick the first account, and follow the redirect back
/// to the callback URL — which is where the code lives.
async fn pick_first_account(authorize_url: &str) -> String {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    let page = http
        .get(authorize_url)
        .send()
        .await
        .expect("authorize page")
        .text()
        .await
        .expect("authorize body");
    let account_id = page
        .split("name=\"account_id\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("an account to pick")
        .to_owned();
    let redirect_uri = query_param(authorize_url, "redirect_uri").expect("redirect_uri");
    let state = query_param(authorize_url, "state").expect("state");

    let response = http
        .post(authorize_url)
        .form(&[
            ("account_id", account_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("state", state.as_str()),
        ])
        .send()
        .await
        .expect("pick");
    let back = response
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("redirect back")
        .to_str()
        .expect("ascii")
        .to_owned();
    // The state must survive the round trip untouched, or the auth
    // server will reject the callback as forged.
    assert_eq!(
        query_param(&back, "state").as_deref(),
        Some(state.as_str()),
        "state echoed back unchanged"
    );
    back
}

/// Create an account and return its session token.
async fn sign_up(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post("/auth/sign-up/email")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"input":{{"email":"{email}","password":"correct-horse-battery-staple"}}}}"#
                )))
                .unwrap(),
        )
        .await
        .expect("sign up");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")["token"]
        .as_str()
        .expect("token")
        .to_owned()
}

/// Run a whole flow through the mock and return the callback's response.
///
/// TONE3000 is link-only — it publishes no verified address, so it can
/// never mint an account — which is why the mode and the session are
/// parameters rather than assumed.
async fn flow_through(
    app: &axum::Router,
    provider: &str,
    mode: &str,
    session: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::get(format!("/auth/social/{provider}/start?mode={mode}"));
    if let Some(session) = session {
        request = request.header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session}"),
        );
    }
    let start = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .expect("start");
    assert_eq!(start.status(), StatusCode::SEE_OTHER, "{provider} start");
    let back = pick_first_account(&location(&start)).await;
    let query = back.split_once('?').expect("callback query").1.to_owned();
    let mut callback = Request::get(format!("/auth/social/{provider}/callback?{query}"));
    if let Some(session) = session {
        callback = callback.header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session}"),
        );
    }
    app.clone()
        .oneshot(callback.body(Body::empty()).unwrap())
        .await
        .expect("callback")
}

#[tokio::test]
async fn a_mock_url_replaces_the_provider_origin_in_the_authorize_redirect() {
    let mock = mock_origin().await;
    let app = app(&mock).await;

    let session = sign_up(&app, "linker@local.test").await;
    for provider in ["github", "google", "tone3000"] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/auth/social/{provider}/start?mode=link"))
                    .header(
                        axum::http::header::AUTHORIZATION,
                        format!("Bearer {session}"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("start");
        let url = location(&response);
        assert!(
            url.starts_with(&format!("{mock}/{provider}/authorize")),
            "{provider} went to {url}"
        );
    }
}

#[tokio::test]
async fn github_and_google_sign_in_end_to_end_through_the_mock() {
    let mock = mock_origin().await;
    let app = app(&mock).await;

    for provider in ["github", "google"] {
        let response = flow_through(&app, provider, "sign-in", None).await;
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "{provider} callback"
        );
        let landed = location(&response);
        assert!(
            !landed.contains("error="),
            "{provider} callback failed: {landed}"
        );
        assert!(
            response
                .headers()
                .get_all(axum::http::header::SET_COOKIE)
                .iter()
                .any(|value| value.to_str().unwrap_or_default().contains("session")),
            "{provider} set no session cookie"
        );
    }
}

#[tokio::test]
async fn tone3000_links_to_an_existing_account() {
    let mock = mock_origin().await;
    let app = app(&mock).await;
    let session = sign_up(&app, "tinkerer@local.test").await;

    let response = flow_through(&app, "tone3000", "link", Some(&session)).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let landed = location(&response);
    assert!(!landed.contains("error="), "link failed: {landed}");

    // The link is only real if the account page now names it. The
    // rendered pages read the cookie, not the bearer header — a bearer
    // here silently redirects to the sign-in page and the assertion
    // below would be checking an empty body.
    let account = app
        .clone()
        .oneshot(
            Request::get("/account")
                .header(
                    axum::http::header::COOKIE,
                    format!("architect-auth.session={session}"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("account page");
    let body = axum::body::to_bytes(account.into_body(), usize::MAX)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("tone-tinkerer"),
        "the linked TONE3000 account is not shown"
    );
}

#[tokio::test]
async fn a_github_address_comes_from_the_second_call_not_the_profile() {
    // GitHub's `/user` leaves `email` null for anyone who keeps it
    // private, and the mock does the same — so an account carrying an
    // address here proves `/user/emails` was fetched and its primary
    // entry chosen over the unverified alternate beside it.
    let mock = mock_origin().await;
    let app = app(&mock).await;

    let response = flow_through(&app, "github", "sign-in", None).await;
    let session = response
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .find_map(|value| {
            value
                .to_str()
                .ok()?
                .strip_prefix("architect-auth.session=")?
                .split(';')
                .next()
                .map(str::to_owned)
        })
        .expect("a session cookie");

    let profile = app
        .clone()
        .oneshot(
            Request::get("/account/profile")
                .header(
                    axum::http::header::COOKIE,
                    format!("architect-auth.session={session}"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("profile");
    let body = axum::body::to_bytes(profile.into_body(), usize::MAX)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("octocat@github.local"),
        "the primary address was not taken from /user/emails"
    );
    assert!(
        !html.contains("alt+octocat@github.local"),
        "the unverified alternate address was chosen"
    );
}

#[tokio::test]
async fn a_code_cannot_be_spent_twice() {
    let mock = mock_origin().await;
    let app = app(&mock).await;

    let start = app
        .clone()
        .oneshot(
            Request::get("/auth/social/google/start?mode=sign-in")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("start");
    let back = pick_first_account(&location(&start)).await;
    let query = back.split_once('?').expect("query").1.to_owned();

    let first = app
        .clone()
        .oneshot(
            Request::get(format!("/auth/social/google/callback?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("first");
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    assert!(!location(&first).contains("error="));

    // The state is single-use too, so this would fail even against a
    // permissive provider — what matters is that it fails.
    let second = app
        .clone()
        .oneshot(
            Request::get(format!("/auth/social/google/callback?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("second");
    assert!(
        location(&second).contains("error="),
        "a replayed code was accepted"
    );
}
