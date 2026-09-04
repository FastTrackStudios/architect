//! Social sign-in, account linking and the relying-party linked-token
//! endpoint, driven through the real router over in-memory SQLite with
//! the provider network swapped for a fake.

use std::sync::{Arc, Mutex};

use architect_auth::AuthStorage;
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use auth_server::http::SocialState;
use auth_server::social::{Profile, Provider, ProviderClient, ProviderError, ProviderTokens};
use auth_server::{ServerConfig, SocialConfig, SocialProviderConfig, server};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tower::ServiceExt;

const BASE: &str = "https://auth.fasttrackstudio.app";
const GITHUB_TOKEN: &str = "gho_plaintext_provider_token";

fn test_config() -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        secret: "a-secret-at-least-32-bytes-long!!".into(),
        base_url: BASE.into(),
        oidc_issuer: None,
        session_ttl_seconds: 3600,
        require_email_verification: false,
        passkey_rp_id: None,
        cors_origins: Vec::new(),
        oidc_clients: vec![architect_auth::OidcClientConfig {
            client_id: "task".into(),
            client_secret: None,
            name: "Task".into(),
            redirect_uris: vec!["https://task.fasttrackstudio.app/auth/callback".into()],
            scopes: vec![
                "openid".into(),
                "email".into(),
                "profile".into(),
                "forge:github".into(),
            ],
            public_client: true,
            skip_consent: true,
            disabled: false,
        }],
        oidc_allow_dynamic_client_registration: false,
        run_migrations: true,
        mail: auth_server::mail::MailConfig {
            host: None,
            port: 587,
            username: None,
            password: None,
            from: "noreply@example.com".into(),
            base_url: BASE.into(),
        },
        social: SocialConfig::disabled(),
    }
}

fn with_providers(config: &mut ServerConfig) {
    config.social.github = Some(SocialProviderConfig {
        client_id: "gh-client-id".into(),
        client_secret: "gh-client-secret".into(),
        scopes: vec!["repo".into(), "read:user".into(), "user:email".into()],
    });
    config.social.google = Some(SocialProviderConfig {
        client_id: "google-client-id".into(),
        client_secret: "google-client-secret".into(),
        scopes: vec!["openid".into(), "email".into(), "profile".into()],
    });
}

/// A provider that never touches the network. Every exchange yields the
/// same token; the profile is whatever the test set.
#[derive(Default)]
struct FakeProvider {
    profile: Mutex<Profile>,
    exchanges: Mutex<Vec<(Provider, String, String)>>,
    profile_fetches: Mutex<Vec<String>>,
}

impl FakeProvider {
    fn octocat() -> Self {
        Self {
            profile: Mutex::new(Profile {
                account_id: "583231".into(),
                email: Some("octocat@github.com".into()),
                email_verified: true,
                name: Some("The Octocat".into()),
                image: None,
                login: Some("octocat".into()),
            }),
            ..Self::default()
        }
    }
}

#[async_trait::async_trait]
impl ProviderClient for FakeProvider {
    async fn exchange_code(
        &self,
        provider: Provider,
        _config: &SocialProviderConfig,
        code: &str,
        redirect_uri: &str,
    ) -> Result<ProviderTokens, ProviderError> {
        self.exchanges
            .lock()
            .unwrap()
            .push((provider, code.to_owned(), redirect_uri.to_owned()));
        if code == "bad-code" {
            return Err(ProviderError::Exchange("bad_verification_code".into()));
        }
        Ok(ProviderTokens {
            access_token: GITHUB_TOKEN.into(),
            refresh_token: None,
            id_token: None,
            expires_in: None,
            scope: Some("repo,read:user,user:email".into()),
        })
    }

    async fn fetch_profile(
        &self,
        _provider: Provider,
        access_token: &str,
    ) -> Result<Profile, ProviderError> {
        self.profile_fetches
            .lock()
            .unwrap()
            .push(access_token.to_owned());
        Ok(self.profile.lock().unwrap().clone())
    }
}

struct Harness {
    app: axum::Router,
    storage: AuthSeaOrmStorage,
    provider: Arc<FakeProvider>,
}

async fn harness(mutate: impl FnOnce(&mut ServerConfig)) -> Harness {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("migrate");
    let mut config = test_config();
    mutate(&mut config);
    let storage = AuthSeaOrmStorage::new(db);
    let auth = server::build_engine(&config, storage.clone()).expect("build engine");
    let provider = Arc::new(FakeProvider::octocat());
    let social = Arc::new(SocialState {
        config: config.social.clone(),
        client: provider.clone(),
        base_url: config.base_url.clone(),
        allowed_return_origins: vec!["https://task.fasttrackstudio.app".into()],
    });
    let app = server::app_router_with_social(&config, auth, social);
    Harness {
        app,
        storage,
        provider,
    }
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("body is json")
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    String::from_utf8_lossy(&bytes).into_owned()
}

fn location(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .expect("a Location header")
        .to_str()
        .expect("ascii")
        .to_owned()
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then(|| percent_decode(v))
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// Sign up with a password; returns the session token.
async fn sign_up(app: &axum::Router, email: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post("/auth/sign-up/email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"email":"{email}","password":"correct-horse-battery-staple"}}"#
                )))
                .unwrap(),
        )
        .await
        .expect("sign up");
    assert_eq!(response.status(), StatusCode::CREATED);
    body_json(response).await["token"]
        .as_str()
        .expect("token")
        .to_owned()
}

/// Run the OIDC code flow for the `task` client and return an access
/// token granted `scope`.
async fn oidc_access_token(app: &axum::Router, session: &str, scope: &str) -> String {
    let verifier = "correct-horse-battery-staple-verifier";
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(<sha2::Sha256 as sha2::Digest>::digest(verifier.as_bytes()));
    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/oauth2/authorize?client_id=task\
                 &redirect_uri=https%3A%2F%2Ftask.fasttrackstudio.app%2Fauth%2Fcallback\
                 &response_type=code&scope={}&state=s\
                 &code_challenge={challenge}&code_challenge_method=S256",
                scope.replace(' ', "%20")
            ))
            .header(header::AUTHORIZATION, format!("Bearer {session}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .expect("authorize");
    assert_eq!(response.status(), StatusCode::SEE_OTHER, "authorize");
    let code = query_param(&location(&response), "code").expect("code");

    let response = app
        .clone()
        .oneshot(
            Request::post("/oauth2/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "grant_type=authorization_code&client_id=task&code={code}\
                     &redirect_uri=https%3A%2F%2Ftask.fasttrackstudio.app%2Fauth%2Fcallback\
                     &code_verifier={verifier}"
                )))
                .unwrap(),
        )
        .await
        .expect("token");
    assert_eq!(response.status(), StatusCode::OK, "token exchange");
    body_json(response).await["access_token"]
        .as_str()
        .expect("access_token")
        .to_owned()
}

/// Start a flow and return the `state` the provider would echo back.
async fn start(app: &axum::Router, provider: &str, mode: &str, session: Option<&str>) -> String {
    let mut request = Request::get(format!(
        "/auth/social/{provider}/start?mode={mode}&return_to=%2Faccount"
    ));
    if let Some(session) = session {
        request = request.header(header::AUTHORIZATION, format!("Bearer {session}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .expect("start");
    assert_eq!(response.status(), StatusCode::SEE_OTHER, "start redirects");
    query_param(&location(&response), "state").expect("state in authorize url")
}

async fn callback(app: &axum::Router, provider: &str, query: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::get(format!("/auth/social/{provider}/callback?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("callback")
}

// ── Off by default ───────────────────────────────────────────────────

#[tokio::test]
async fn with_no_providers_the_routes_404_and_the_pages_show_no_buttons() {
    let h = harness(|_| {}).await;

    for path in [
        "/auth/social/github/start",
        "/auth/social/google/start?mode=sign-in",
        "/auth/social/github/callback?code=x&state=y",
        "/auth/social/facebook/start",
    ] {
        let response = h
            .app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .expect("request");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }

    for path in ["/login", "/sign-up"] {
        let response = h
            .app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .expect("page");
        let html = body_text(response).await;
        assert!(
            !html.contains("Continue with"),
            "{path} must show no social buttons: {html}"
        );
    }
}

// ── Starting a flow ──────────────────────────────────────────────────

#[tokio::test]
async fn start_redirects_to_the_provider_with_client_callback_scope_and_state() {
    let h = harness(with_providers).await;

    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/social/github/start?mode=sign-in&return_to=%2Faccount")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("start");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let url = location(&response);
    assert!(
        url.starts_with("https://github.com/login/oauth/authorize?"),
        "{url}"
    );
    assert_eq!(
        query_param(&url, "client_id").as_deref(),
        Some("gh-client-id")
    );
    assert_eq!(
        query_param(&url, "redirect_uri").as_deref(),
        Some("https://auth.fasttrackstudio.app/auth/social/github/callback")
    );
    assert_eq!(
        query_param(&url, "scope").as_deref(),
        Some("repo read:user user:email")
    );
    let state = query_param(&url, "state").expect("state");
    assert!(
        state.len() > 40,
        "state carries a nonce and a sealed payload"
    );
    assert!(
        !state.contains("account"),
        "the pending flow must not be readable from the state: {state}"
    );

    // Google's authorize URL, and its link-only offline_access.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/social/google/start?mode=sign-in")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("start");
    let url = location(&response);
    assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
    assert_eq!(
        query_param(&url, "prompt").as_deref(),
        Some("select_account")
    );
    assert!(query_param(&url, "access_type").is_none());
}

#[tokio::test]
async fn the_pages_offer_the_configured_providers() {
    let h = harness(with_providers).await;
    for path in [
        "/login?return_to=%2Foauth2%2Fauthorize%3Fclient_id%3Dtask",
        "/sign-up",
    ] {
        let response = h
            .app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .expect("page");
        let html = body_text(response).await;
        assert!(html.contains("Continue with GitHub"), "{path}: {html}");
        assert!(html.contains("Continue with Google"), "{path}: {html}");
        assert!(
            html.contains("/auth/social/github/start?mode=sign-in&#38;return_to="),
            "{path}: {html}"
        );
    }
}

#[tokio::test]
async fn linking_requires_a_session() {
    let h = harness(with_providers).await;

    // A browser is sent to sign in, and comes back here afterwards.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/social/github/start?mode=link&return_to=%2Faccount")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("start");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let url = location(&response);
    assert!(url.starts_with("/login?return_to="), "{url}");
    assert!(url.contains("%2Fauth%2Fsocial%2Fgithub%2Fstart"), "{url}");

    // A program gets a 401.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/social/github/start?mode=link")
                .header(header::AUTHORIZATION, "Bearer not-a-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("start");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_hostile_return_to_falls_back_to_the_account_page() {
    let h = harness(with_providers).await;
    let session = sign_up(&h.app, "cody@fasttrackstudio.app").await;

    for (raw, expected) in [
        ("https://evil.example/steal", "/account"),
        ("//evil.example", "/account"),
        ("javascript:alert(1)", "/account"),
        // Same origin as this server: allowed.
        (
            "https://auth.fasttrackstudio.app/somewhere",
            "https://auth.fasttrackstudio.app/somewhere",
        ),
        // A registered OIDC client's origin: allowed.
        (
            "https://task.fasttrackstudio.app/settings",
            "https://task.fasttrackstudio.app/settings",
        ),
    ] {
        let encoded = raw
            .replace(':', "%3A")
            .replace('/', "%2F")
            .replace('(', "%28")
            .replace(')', "%29");
        let response = h
            .app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/auth/social/github/start?mode=link&return_to={encoded}"
                ))
                .header(header::AUTHORIZATION, format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .expect("start");
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{raw}");
        let state = query_param(&location(&response), "state").expect("state");
        // The callback lands where the sealed flow says, so bounce the
        // state straight back with a denied error and read the redirect.
        let response = callback(
            &h.app,
            "github",
            &format!("error=access_denied&state={}", state.replace('+', "%2B")),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{raw}");
        assert_eq!(
            location(&response),
            format!("{expected}?error=denied"),
            "{raw}"
        );
    }
}

// ── The callback ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_callback_with_an_unknown_state_redirects_with_error_state() {
    let h = harness(with_providers).await;

    let response = callback(&h.app, "github", "code=abc&state=not-a-real-state").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/login?error=state");

    // A well-formed state cannot be used twice: the nonce is consumed.
    let state = start(&h.app, "github", "sign-in", None).await;
    let first = callback(&h.app, "github", &format!("code=abc&state={state}")).await;
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    assert!(
        !location(&first).contains("error=state"),
        "first use must pass the state check: {}",
        location(&first)
    );
    let replay = callback(&h.app, "github", &format!("code=abc&state={state}")).await;
    assert_eq!(location(&replay), "/account?error=state");
}

#[tokio::test]
async fn a_refused_code_redirects_with_error_exchange() {
    let h = harness(with_providers).await;
    let state = start(&h.app, "github", "sign-in", None).await;
    let response = callback(&h.app, "github", &format!("code=bad-code&state={state}")).await;
    assert_eq!(location(&response), "/account?error=exchange");
}

#[tokio::test]
async fn social_sign_in_creates_the_user_and_sets_the_session_cookie() {
    let h = harness(with_providers).await;
    let state = start(&h.app, "github", "sign-in", None).await;

    let response = callback(&h.app, "github", &format!("code=good&state={state}")).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/account");
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("architect-auth.session="));
    assert!(cookie.contains("HttpOnly"));

    // The exchange went to the fake with the right callback URL.
    let exchanges = h.provider.exchanges.lock().unwrap().clone();
    assert_eq!(exchanges.len(), 1);
    assert_eq!(exchanges[0].0, Provider::GitHub);
    assert_eq!(exchanges[0].1, "good");
    assert_eq!(
        exchanges[0].2,
        "https://auth.fasttrackstudio.app/auth/social/github/callback"
    );

    // The cookie is a real session for the new user.
    let session_cookie = cookie.split(';').next().unwrap().to_owned();
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/session")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("session");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await["user"]["email"],
        "octocat@github.com"
    );

    // Signing in again with the same GitHub identity resumes the same
    // user rather than failing on the existing email.
    let state = start(&h.app, "github", "sign-in", None).await;
    let response = callback(&h.app, "github", &format!("code=good&state={state}")).await;
    assert_eq!(location(&response), "/account");
}

#[tokio::test]
async fn social_sign_in_with_an_email_that_has_a_password_account_is_refused_not_merged() {
    // The engine does not link by email: a provider saying "this is
    // cody@…" is not proof of owning the password account with that
    // address. The person is told to sign in and link instead.
    let h = harness(with_providers).await;
    sign_up(&h.app, "octocat@github.com").await;

    let state = start(&h.app, "github", "sign-in", None).await;
    let response = callback(&h.app, "github", &format!("code=good&state={state}")).await;
    assert_eq!(location(&response), "/account?error=email_in_use");
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}

// ── Linking, and the token that comes out the other end ──────────────

#[tokio::test]
async fn linking_stores_ciphertext_and_linked_token_returns_the_plaintext() {
    let h = harness(with_providers).await;
    let session = sign_up(&h.app, "cody@fasttrackstudio.app").await;

    // Before linking: the relying party is told so.
    let rp_token = oidc_access_token(&h.app, &session, "openid email forge:github").await;
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/oauth2/linked-token?provider=github")
                .header(header::AUTHORIZATION, format!("Bearer {rp_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("linked-token");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(response).await["error"], "not_linked");

    // Link.
    let state = start(&h.app, "github", "link", Some(&session)).await;
    let response = callback(&h.app, "github", &format!("code=good&state={state}")).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/account?linked=github");

    // What hit the database is not the token.
    let row = h
        .storage
        .find_account_by_provider_account("github", "583231")
        .await
        .expect("query")
        .expect("a linked row");
    let stored = row
        .access_token_ciphertext
        .as_deref()
        .expect("token stored");
    assert_ne!(stored, GITHUB_TOKEN);
    assert!(
        !stored.contains(GITHUB_TOKEN),
        "plaintext must not be stored"
    );
    assert!(stored.starts_with("v2."), "engine envelope: {stored}");
    assert_eq!(row.scope.as_deref(), Some("repo,read:user,user:email"));

    // The owner's view never includes the token.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/accounts")
                .header(header::AUTHORIZATION, format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("accounts");
    assert_eq!(response.status(), StatusCode::OK);
    let accounts = body_json(response).await;
    let list = accounts.as_array().expect("a list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["provider_id"], "github");
    assert_eq!(list[0]["account_id"], "583231");
    assert_eq!(list[0]["login"], "octocat");
    assert!(list[0].get("access_token").is_none());
    assert!(!accounts.to_string().contains(GITHUB_TOKEN));

    // The account page shows it linked.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/account")
                .header(header::AUTHORIZATION, format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("account page");
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("Linked as"), "{html}");
    assert!(html.contains("octocat"), "{html}");
    assert!(html.contains("Link Google"), "{html}");
    assert!(html.contains("Unlink"), "{html}");
    assert!(!html.contains(GITHUB_TOKEN));

    // The relying party gets the plaintext back.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/oauth2/linked-token?provider=github")
                .header(header::AUTHORIZATION, format!("Bearer {rp_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("linked-token");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["provider"], "github");
    assert_eq!(body["login"], "octocat");
    assert_eq!(body["account_id"], "583231");
    assert_eq!(body["access_token"], GITHUB_TOKEN);
    assert_eq!(body["scope"], "repo,read:user,user:email");

    // Unlinking is allowed: the password remains as a way in.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::post("/auth/accounts/github/unlink")
                .header(header::AUTHORIZATION, format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("unlink");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        h.storage
            .find_account_by_provider_account("github", "583231")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn linked_token_refuses_a_token_without_the_scope_and_a_bad_bearer() {
    let h = harness(with_providers).await;
    let session = sign_up(&h.app, "cody@fasttrackstudio.app").await;

    let plain = oidc_access_token(&h.app, &session, "openid email").await;
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/oauth2/linked-token?provider=github")
                .header(header::AUTHORIZATION, format!("Bearer {plain}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("linked-token");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(response).await["error"], "insufficient_scope");

    // No bearer, and a garbage bearer, are both 401.
    for auth in [None, Some("Bearer not-a-jwt")] {
        let mut request = Request::get("/oauth2/linked-token");
        if let Some(auth) = auth {
            request = request.header(header::AUTHORIZATION, auth);
        }
        let response = h
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("linked-token");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{auth:?}");
    }

    // A session token is not an access token. It must not work here even
    // though it identifies the same person: the scope gate lives on the
    // OIDC grant, and a session has no grant.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/oauth2/linked-token")
                .header(header::AUTHORIZATION, format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("linked-token");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Discovery advertises the scope, so a client can find it.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/.well-known/openid-configuration")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("discovery");
    let scopes = body_json(response).await["scopes_supported"].clone();
    assert!(
        scopes
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "forge:github"),
        "{scopes}"
    );
}

#[tokio::test]
async fn unlinking_the_only_credential_is_refused_with_409() {
    let h = harness(with_providers).await;

    // A user who only ever signed in with GitHub.
    let state = start(&h.app, "github", "sign-in", None).await;
    let response = callback(&h.app, "github", &format!("code=good&state={state}")).await;
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    let response = h
        .app
        .clone()
        .oneshot(
            Request::post("/auth/accounts/github/unlink")
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("unlink");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "last_credential");

    // The page form says the same thing, in words.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::post("/account/unlink")
                .header(header::COOKIE, cookie.clone())
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("provider=github"))
                .unwrap(),
        )
        .await
        .expect("unlink form");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/account?error=last_credential");

    // A sign-in-only link holds no token for a relying party.
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/session")
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("session");
    assert_eq!(response.status(), StatusCode::OK);
    let row = h
        .storage
        .find_account_by_provider_account("github", "583231")
        .await
        .unwrap()
        .expect("row");
    assert!(row.access_token_ciphertext.is_none());
}

#[tokio::test]
async fn the_account_page_needs_a_session_and_unlinking_an_unlinked_provider_is_404() {
    let h = harness(with_providers).await;

    let response = h
        .app
        .clone()
        .oneshot(Request::get("/account").body(Body::empty()).unwrap())
        .await
        .expect("account");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/login?return_to=%2Faccount");

    let session = sign_up(&h.app, "cody@fasttrackstudio.app").await;
    let response = h
        .app
        .clone()
        .oneshot(
            Request::post("/auth/accounts/google/unlink")
                .header(header::AUTHORIZATION, format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("unlink");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(response).await["error"], "not_linked");
}
