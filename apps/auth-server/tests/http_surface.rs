//! End-to-end coverage of the HTTP surface against a real engine over
//! in-memory SQLite.
//!
//! These drive the actual `app_router`, so they exercise routing,
//! extraction, cookie shaping and error mapping together — the parts a
//! compile check cannot vouch for.

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
        database_url: "sqlite::memory:".into(),
        secret: "a-secret-at-least-32-bytes-long!!".into(),
        base_url: "https://auth.fasttrackstudio.app".into(),
        oidc_issuer: None,
        session_ttl_seconds: 3600,
        require_email_verification: false,
        passkey_rp_id: None,
        cors_origins: Vec::new(),
        oidc_clients: Vec::new(),
        oidc_allow_dynamic_client_registration: false,
        run_migrations: true,
    }
}

/// Build the app with a config the caller can adjust first.
async fn app_with(mutate: impl FnOnce(&mut ServerConfig)) -> axum::Router {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("migrate");
    let mut config = test_config();
    mutate(&mut config);
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    server::app_router(&config, auth)
}

async fn app() -> axum::Router {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("migrate");
    let config = test_config();
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db)).expect("build engine");
    server::app_router(&config, auth)
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("body is json")
}

#[tokio::test]
async fn discovery_advertises_the_configured_issuer() {
    let response = app()
        .await
        .oneshot(
            Request::get("/.well-known/openid-configuration")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["issuer"], "https://auth.fasttrackstudio.app");
    assert_eq!(
        body["token_endpoint"],
        "https://auth.fasttrackstudio.app/oauth2/token"
    );
    // The advertised jwks_uri must be a path this server actually
    // mounts, or every RP's discovery step dead-ends on a 404.
    assert_eq!(
        body["jwks_uri"],
        "https://auth.fasttrackstudio.app/auth/jwt/jwks"
    );
}

#[tokio::test]
async fn advertised_jwks_uri_is_routed_and_leaks_no_key_material() {
    let response = app()
        .await
        .oneshot(Request::get("/auth/jwt/jwks").body(Body::empty()).unwrap())
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    // HS256 means the "public" key is the signing secret. An empty set
    // is correct; a populated one would be a disclosure.
    assert_eq!(body["keys"].as_array().expect("keys array").len(), 0);
    let serialized = body.to_string();
    assert!(
        !serialized.contains("a-secret-at-least-32-bytes-long"),
        "jwks must never contain the signing secret"
    );
}

#[tokio::test]
async fn sign_up_then_session_round_trips_a_bearer_token() {
    let app = app().await;

    let response = app
        .clone()
        .oneshot(
            Request::post("/auth/sign-up/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"email":"cody@fasttrackstudio.app","password":"correct-horse-battery-staple"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("sign up");

    assert_eq!(response.status(), StatusCode::CREATED);
    // The browser path: a session cookie is set on the response.
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie is set")
        .to_str()
        .expect("cookie is ascii")
        .to_owned();
    assert!(cookie.contains("architect-auth.session="));
    assert!(
        cookie.contains("HttpOnly"),
        "session cookie must be HttpOnly"
    );
    assert!(
        cookie.contains("Secure"),
        "an https base_url must yield a Secure cookie"
    );

    let body = json_body(response).await;
    let token = body["token"].as_str().expect("token in body").to_owned();
    assert_eq!(body["user"]["email"], "cody@fasttrackstudio.app");
    // The stored verifier must never be serialized.
    assert!(body["session"].get("token_hash").is_none());

    // The native path: the same token as a bearer resolves the session.
    let response = app
        .oneshot(
            Request::get("/auth/session")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("session");

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["user"]["email"], "cody@fasttrackstudio.app");
}

#[tokio::test]
async fn session_without_credentials_is_401_not_500() {
    let response = app()
        .await
        .oneshot(Request::get("/auth/session").body(Body::empty()).unwrap())
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_password_is_indistinguishable_from_unknown_account() {
    let app = app().await;

    app.clone()
        .oneshot(
            Request::post("/auth/sign-up/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"email":"real@fasttrackstudio.app","password":"correct-horse-battery-staple"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("sign up");

    let wrong_password = app
        .clone()
        .oneshot(
            Request::post("/auth/sign-in/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"email":"real@fasttrackstudio.app","password":"not-the-password"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("sign in");

    let unknown_account = app
        .oneshot(
            Request::post("/auth/sign-in/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"email":"ghost@fasttrackstudio.app","password":"not-the-password"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("sign in");

    assert_eq!(wrong_password.status(), unknown_account.status());
    assert_eq!(
        json_body(wrong_password).await,
        json_body(unknown_account).await,
        "a differing response would let an attacker enumerate accounts"
    );
}

#[tokio::test]
async fn sign_out_is_idempotent_and_clears_the_cookie() {
    let response = app()
        .await
        .oneshot(Request::post("/auth/sign-out").body(Body::empty()).unwrap())
        .await
        .expect("request");

    // No token supplied: already signed out, so this is a success.
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("cookie cleared")
        .to_str()
        .unwrap();
    assert!(cookie.contains("architect-auth.session="));
}

#[tokio::test]
async fn health_probes_answer() {
    let app = app().await;
    for path in ["/healthz", "/readyz"] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .expect("probe");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn configuring_cors_origins_does_not_panic_and_echoes_the_origin() {
    // Regression: the layer combined `allow_credentials(true)` with
    // wildcard methods/headers. That is forbidden, and tower-http
    // enforces it by panicking *when the router is built* — so the
    // server started fine with no origins configured and died on boot
    // the moment a deployment set AUTH_CORS_ORIGINS, which every real
    // deployment does.
    let app = app_with(|config| {
        config.cors_origins = vec!["https://keyflow.fasttrackstudio.app".into()];
    })
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/auth/sign-in/email")
                .header(header::ORIGIN, "https://keyflow.fasttrackstudio.app")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "content-type")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("preflight");

    let headers = response.headers();
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("https://keyflow.fasttrackstudio.app")
    );
    // Credentials must stay on, so a session cookie can ride a
    // cross-origin request from a front-end on another host.
    assert_eq!(
        headers
            .get("access-control-allow-credentials")
            .and_then(|value| value.to_str().ok()),
        Some("true")
    );
}

#[tokio::test]
async fn an_unlisted_origin_is_not_granted_cors_access() {
    let app = app_with(|config| {
        config.cors_origins = vec!["https://keyflow.fasttrackstudio.app".into()];
    })
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/auth/sign-in/email")
                .header(header::ORIGIN, "https://evil.example")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("preflight");

    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none(),
        "an unlisted origin must not be echoed back"
    );
}

// ── The browser redirect flow ────────────────────────────────────────

/// The change that makes `/oauth2/authorize` usable from a browser at
/// all. It used to answer 401 for everyone, so an app that sent someone
/// here to sign in handed them a JSON error instead of a login page.
#[tokio::test]
async fn authorize_without_a_session_sends_a_browser_to_the_login_page() {
    let response = app()
        .await
        .oneshot(
            Request::get(
                "/oauth2/authorize?client_id=task\
                 &redirect_uri=https://task.fasttrackstudio.app/auth/callback\
                 &response_type=code&scope=openid&state=xyz",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get(header::LOCATION)
        .expect("a Location header")
        .to_str()
        .expect("ascii");
    assert!(
        location.starts_with("/login?return_to="),
        "expected the login page, got {location}"
    );
    // The whole original request has to survive the round trip, or
    // signing in resumes a request that has lost its parameters. The
    // `&` separators must be encoded, or `return_to` ends at the first
    // one and everything after client_id is silently dropped.
    assert!(
        location.contains("%26response_type%3Dcode"),
        "the authorize query must be encoded into return_to, got {location}"
    );
    assert!(
        location.contains("state%3Dxyz"),
        "the tail of the query must survive, got {location}"
    );
}

/// A program is not a person: it cannot render a login page, and a 303
/// to HTML would read as a baffling success. Bearer callers keep the
/// old 401.
#[tokio::test]
async fn authorize_with_a_bearer_token_still_gets_401() {
    let response = app()
        .await
        .oneshot(
            // The same complete query as the browser case — an
            // incomplete one is rejected by extraction with a 400 before
            // the handler runs, which would make this test pass for the
            // wrong reason.
            Request::get(
                "/oauth2/authorize?client_id=task\
                 &redirect_uri=https://task.fasttrackstudio.app/auth/callback\
                 &response_type=code&scope=openid&state=xyz",
            )
            .header(header::AUTHORIZATION, "Bearer not-a-real-token")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The pages have to actually be mounted — the whole redirect is a dead
/// end if `/login` 404s.
#[tokio::test]
async fn the_sign_in_and_sign_up_pages_are_served() {
    for path in ["/login", "/sign-up"] {
        let response = app()
            .await
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .expect("request");

        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(
            content_type.starts_with("text/html"),
            "{path} should serve HTML, got {content_type}"
        );
    }
}
