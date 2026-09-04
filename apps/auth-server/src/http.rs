//! The HTTP surface: the OIDC provider endpoints plus a small
//! JSON session API.
//!
//! ## Why this module exists
//!
//! `architect-auth`'s `transport::axum` module ships route *descriptors*
//! and a `require_session` middleware, but it never builds a `Router` —
//! there is not one `.route()` call in the engine. Everything a client
//! could reach was therefore reachable only over vox. That is fine for
//! the Rust apps, and useless for a browser hitting
//! `/.well-known/openid-configuration`. This module is the missing
//! mount.
//!
//! ## Scope
//!
//! Deliberately not all ~150 route descriptors. Two surfaces earn their
//! place in a standalone identity server:
//!
//! * **OIDC provider** — discovery, authorize, token, userinfo, JWKS.
//!   This is what makes the server an IdP that anything can point at.
//! * **Core session JSON** — sign-up, sign-in, session, refresh,
//!   sign-out. What a web front-end needs before it can do anything.
//!
//! The remaining commands (admin, orgs, teams, passkeys, 2FA, API keys)
//! stay reachable over vox and through `ArchitectAuth` directly. Adding
//! them here later is purely additive. The blocker on doing it
//! *generically* is that the command structs derive no serde, so each
//! one needs a hand-written extractor — see `docs` in the crate root.

use std::sync::Arc;

use architect_auth::{
    ArchitectAuth, AuthStorage, AuthorizeOidc, BeginOAuthAuthorization, CreateEmailPasswordUser,
    CurrentSession, ExchangeOidcToken, GetOidcUserInfo, LinkOAuthAccount, RefreshSession,
    SignInOAuthAccount, SignOut, UnlinkOAuthAccount, VerifyJwt, VerifyOAuthState,
    crypto::decrypt_secret,
    transport::{AuthCookieConfig, axum::session_token_from_headers, map_auth_error},
};
use auth_proto::{AuthAccount, AuthFlowError, AuthSessionBundle, AuthUser, SignInEmailPassword};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde_json::{Value, json};

use crate::config::{SocialConfig, SocialProviderConfig};
use crate::social::{
    HttpProviderClient, Mode, PendingFlow, Provider, ProviderClient, email_from_id_token,
    encode_component,
};

/// Everything the HTTP handlers need: the auth engine plus the cookie
/// policy that names the session cookie.
#[derive(Clone)]
pub struct HttpState<S> {
    pub auth: ArchitectAuth<S>,
    pub cookie: AuthCookieConfig,
    /// Outgoing mail. Defaults to a log-only mailer so every existing
    /// caller — and every test — keeps working without one; a deployment
    /// supplies the real thing with [`HttpState::with_mailer`].
    pub mail: std::sync::Arc<crate::mail::Mailer>,
    /// Social providers. Defaults to none configured, which turns the
    /// `/auth/social/*` routes into 404s and hides the buttons.
    pub social: Arc<SocialState>,
}

/// Everything the social routes need beyond the engine.
pub struct SocialState {
    pub config: SocialConfig,
    /// The network half, swappable for a fake in tests.
    pub client: Arc<dyn ProviderClient>,
    /// The public origin, for building callback URLs and for deciding
    /// what `return_to` may point at.
    pub base_url: String,
    /// Origins of every registered OIDC client's redirect URIs. A
    /// `return_to` is allowed to land on one of these, because that is
    /// exactly where a client that started the flow lives.
    pub allowed_return_origins: Vec<String>,
}

impl SocialState {
    pub fn disabled() -> Self {
        Self {
            config: SocialConfig::disabled(),
            client: Arc::new(NoProviderClient),
            base_url: "http://localhost:8080".into(),
            allowed_return_origins: Vec::new(),
        }
    }

    pub fn provider_config(&self, provider: Provider) -> Option<&SocialProviderConfig> {
        match provider {
            Provider::GitHub => self.config.github.as_ref(),
            Provider::Google => self.config.google.as_ref(),
        }
    }

    /// The providers that are actually on, in button order.
    pub fn enabled_providers(&self) -> Vec<Provider> {
        Provider::ALL
            .into_iter()
            .filter(|p| self.provider_config(*p).is_some())
            .collect()
    }

    pub fn callback_url(&self, provider: Provider) -> String {
        format!(
            "{}/auth/social/{}/callback",
            self.base_url.trim_end_matches('/'),
            provider.id()
        )
    }

    /// Reduce a caller-supplied `return_to` to something safe to send a
    /// freshly signed-in browser to.
    ///
    /// Same-origin paths always pass. An absolute URL passes only when
    /// its origin is this server's or a registered OIDC client's — the
    /// places a flow can legitimately have started from. Anything else
    /// is an open redirect on an identity server, and lands on
    /// `/account` instead.
    pub fn safe_return_to(&self, raw: Option<&str>) -> String {
        const DEFAULT: &str = "/account";
        let candidate = raw.unwrap_or("").trim();
        if candidate.is_empty() || candidate.contains(['\\', '\r', '\n', ' ']) {
            return DEFAULT.to_owned();
        }
        if candidate.starts_with('/') {
            return if candidate.starts_with("//") {
                DEFAULT.to_owned()
            } else {
                candidate.to_owned()
            };
        }
        let Some(origin) = origin_of(candidate) else {
            return DEFAULT.to_owned();
        };
        let own = origin_of(&self.base_url);
        if own.as_deref() == Some(origin.as_str())
            || self
                .allowed_return_origins
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&origin))
        {
            candidate.to_owned()
        } else {
            DEFAULT.to_owned()
        }
    }
}

/// `scheme://host[:port]`, lower-cased, of an absolute http(s) URL.
pub(crate) fn origin_of(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .map(|r| ("https://", r))
        .or_else(|| lower.strip_prefix("http://").map(|r| ("http://", r)))?;
    let host = rest.1.split(['/', '?', '#']).next()?;
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!("{}{host}", rest.0))
}

/// The client behind a server with no providers. Never reached — the
/// routes 404 before any exchange — but the state needs *something*.
struct NoProviderClient;

#[async_trait::async_trait]
impl ProviderClient for NoProviderClient {
    async fn exchange_code(
        &self,
        _provider: Provider,
        _config: &SocialProviderConfig,
        _code: &str,
        _redirect_uri: &str,
    ) -> Result<crate::social::ProviderTokens, crate::social::ProviderError> {
        Err(crate::social::ProviderError::Exchange(
            "no provider configured".into(),
        ))
    }

    async fn fetch_profile(
        &self,
        _provider: Provider,
        _access_token: &str,
    ) -> Result<crate::social::Profile, crate::social::ProviderError> {
        Err(crate::social::ProviderError::Profile(
            "no provider configured".into(),
        ))
    }
}

impl<S> HttpState<S> {
    pub fn new(auth: ArchitectAuth<S>, cookie: AuthCookieConfig) -> Self {
        let mail = crate::mail::Mailer::new(crate::mail::MailConfig {
            host: None,
            port: 587,
            username: None,
            password: None,
            from: "noreply@localhost".into(),
            base_url: "http://localhost:8080".into(),
        })
        .expect("a mailer with no host cannot fail to build");
        Self {
            auth,
            cookie,
            mail: std::sync::Arc::new(mail),
            social: Arc::new(SocialState::disabled()),
        }
    }

    /// Attach a configured mailer.
    pub fn with_mailer(mut self, mail: std::sync::Arc<crate::mail::Mailer>) -> Self {
        self.mail = mail;
        self
    }

    /// Attach the social providers.
    pub fn with_social(mut self, social: Arc<SocialState>) -> Self {
        self.social = social;
        self
    }

    /// The real provider client, or a failure to build one (a TLS
    /// backend that refuses to initialise) — which a caller should treat
    /// as fatal, since the alternative is a server that sends people to
    /// GitHub and cannot finish.
    pub fn social_state(
        config: &crate::config::ServerConfig,
    ) -> Result<SocialState, reqwest::Error> {
        Ok(SocialState {
            config: config.social.clone(),
            client: Arc::new(HttpProviderClient::new()?),
            base_url: config.base_url.clone(),
            allowed_return_origins: config
                .oidc_clients
                .iter()
                .flat_map(|client| client.redirect_uris.iter())
                .filter_map(|uri| origin_of(uri))
                .collect(),
        })
    }
}

/// Build the HTTP router.
///
/// Mounted alongside the vox WebSocket by [`crate::server`]; kept
/// separate so a consumer embedding architect-auth in an existing axum
/// app can merge just this.
pub fn router<S>(state: HttpState<S>) -> Router
where
    S: AuthStorage + Clone + Send + Sync + 'static,
{
    Router::new()
        // ── OIDC provider ────────────────────────────────────────────
        .route(
            "/.well-known/openid-configuration",
            get(openid_configuration::<S>),
        )
        // Discovery advertises `jwks_uri` as `/auth/jwt/jwks`, so that
        // is the canonical path. `/oauth2/jwks.json` is an alias because
        // several client libraries guess it.
        .route("/auth/jwt/jwks", get(jwks::<S>))
        .route("/oauth2/jwks.json", get(jwks::<S>))
        .route(
            "/oauth2/authorize",
            get(authorize::<S>).post(authorize::<S>),
        )
        .route("/oauth2/token", post(token::<S>))
        .route("/oauth2/userinfo", get(userinfo::<S>).post(userinfo::<S>))
        // ── Core session JSON ────────────────────────────────────────
        .route("/auth/sign-up/email", post(sign_up_email::<S>))
        .route("/auth/sign-in/email", post(sign_in_email::<S>))
        .route("/auth/session", get(session::<S>))
        .route("/auth/refresh", post(refresh::<S>))
        .route("/auth/sign-out", post(sign_out::<S>))
        // ── Social sign-in and account linking ───────────────────────
        .route("/auth/social/{provider}/start", get(social_start::<S>))
        .route(
            "/auth/social/{provider}/callback",
            get(social_callback::<S>),
        )
        .route("/auth/accounts", get(accounts::<S>))
        .route(
            "/auth/accounts/{provider}/unlink",
            post(unlink_account::<S>),
        )
        // ── Relying-party access to a linked token ───────────────────
        .route("/oauth2/linked-token", get(linked_token::<S>))
        // ── Introspection ────────────────────────────────────────────
        .route("/openapi.json", get(openapi))
        .with_state(state)
}

// ── OIDC ─────────────────────────────────────────────────────────────

async fn openid_configuration<S>(State(state): State<HttpState<S>>) -> Json<Value>
where
    S: AuthStorage,
{
    let discovery = state.auth.oidc_discovery();
    Json(json!({
        "issuer": discovery.issuer,
        "authorization_endpoint": discovery.authorization_endpoint,
        "token_endpoint": discovery.token_endpoint,
        "userinfo_endpoint": discovery.userinfo_endpoint,
        "jwks_uri": discovery.jwks_uri,
        "registration_endpoint": discovery.registration_endpoint,
        "scopes_supported": discovery.scopes_supported,
        "response_types_supported": discovery.response_types_supported,
        "grant_types_supported": discovery.grant_types_supported,
        "token_endpoint_auth_methods_supported": discovery.token_endpoint_auth_methods_supported,
        "id_token_signing_alg_values_supported": discovery.id_token_signing_alg_values_supported,
        "code_challenge_methods_supported": discovery.code_challenge_methods_supported,
        "claims_supported": discovery.claims_supported,
    }))
}

/// JWKS — deliberately **empty** while the engine signs with HS256.
///
/// architect-auth's JWT layer is symmetric (`Algorithm::HS256`,
/// hardcoded). A JWKS is a set of *public* keys; the "public" half of an
/// HS256 key is the signing secret itself. Publishing it here would hand
/// every reader the power to mint tokens for every account.
///
/// So this endpoint returns a valid, empty key set. Relying parties must
/// verify id_tokens by calling `/oauth2/userinfo`, or share the secret
/// out of band (which is acceptable for first-party apps and not for
/// anyone else). Asymmetric signing is the fix; see the crate docs.
async fn jwks<S>(State(state): State<HttpState<S>>) -> Json<Value>
where
    S: AuthStorage,
{
    let descriptors = state.auth.jwt_key_set();
    // Reported so an operator can see which kids exist and which is
    // active, without any key material.
    let kids: Vec<Value> = descriptors
        .keys
        .iter()
        .map(|key| json!({ "kid": key.kid, "alg": key.alg, "active": key.active }))
        .collect();
    Json(json!({
        "keys": [],
        "x-architect-auth-note":
            "This issuer signs with HS256 (symmetric). Publishing key material here \
             would disclose the signing secret, so the key set is empty by design. \
             Verify tokens via the userinfo endpoint.",
        "x-architect-auth-kids": kids,
    }))
}

/// The authorization endpoint.
///
/// The session comes from the cookie (browser) or a bearer header (a
/// native app that already signed in).
///
/// # When there is no session
///
/// A browser is sent to the sign-in page with the whole original request
/// as `return_to`, so signing in resumes the flow instead of ending it.
/// This used to answer `401` regardless, which made the redirect flow
/// unusable from a browser: the app would send someone here to log in
/// and they would receive a JSON error.
///
/// A request that carried an `Authorization` header is answered with
/// `401` as before — it is a program, not a person, and a login page is
/// not something it can render.
async fn authorize<S>(
    State(state): State<HttpState<S>>,
    uri: Uri,
    headers: HeaderMap,
    Query(params): Query<AuthorizeParams>,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    let session_token = match session_token_from_headers(&headers, &state.cookie) {
        Some(token) => token,
        None if headers.contains_key(header::AUTHORIZATION) => {
            return Err(ApiError::from(AuthFlowError::InvalidCredentials));
        }
        None => return Ok(Redirect::to(&sign_in_url(&uri)).into_response()),
    };

    let authorization = state
        .auth
        .authorize_oidc(AuthorizeOidc {
            session_token,
            client_id: params.client_id,
            redirect_uri: params.redirect_uri,
            response_type: params.response_type.unwrap_or_else(|| "code".into()),
            scope: params.scope,
            state: params.state,
            nonce: params.nonce,
            code_challenge: params.code_challenge,
            code_challenge_method: params.code_challenge_method,
            prompt: params.prompt,
        })
        .await?;

    // The engine already appended `code` and `state` to the registered
    // redirect_uri, so this is a plain 302 to a URI it validated.
    Ok(Redirect::to(&authorization.redirect_uri).into_response())
}

#[derive(Debug, serde::Deserialize)]
pub struct AuthorizeParams {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub prompt: Option<String>,
}

/// The token endpoint. Accepts `application/x-www-form-urlencoded`, as
/// RFC 6749 requires.
async fn token<S>(
    State(state): State<HttpState<S>>,
    axum::extract::Form(form): axum::extract::Form<TokenForm>,
) -> Result<Json<Value>, ApiError>
where
    S: AuthStorage,
{
    let response = state
        .auth
        .exchange_oidc_token(ExchangeOidcToken {
            grant_type: form.grant_type,
            code: form.code,
            redirect_uri: form.redirect_uri,
            client_id: form.client_id,
            client_secret: form.client_secret,
            code_verifier: form.code_verifier,
            refresh_token: form.refresh_token,
        })
        .await?;

    Ok(Json(json!({
        "access_token": response.access_token,
        "id_token": response.id_token,
        "refresh_token": response.refresh_token,
        "token_type": response.token_type,
        "expires_in": response.expires_in,
        "scope": response.scope,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    pub client_id: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

async fn userinfo<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError>
where
    S: AuthStorage,
{
    let access_token = bearer(&headers).ok_or(ApiError::from(AuthFlowError::InvalidCredentials))?;
    let info = state
        .auth
        .get_oidc_user_info(GetOidcUserInfo { access_token })
        .await?;
    Ok(Json(json!({
        "sub": info.sub,
        "email": info.email,
        "email_verified": info.email_verified,
        "name": info.name,
        "picture": info.picture,
    })))
}

// ── Session JSON ─────────────────────────────────────────────────────

async fn sign_up_email<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Json(body): Json<SignUpBody>,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    let bundle = state
        .auth
        .create_email_password_user(CreateEmailPasswordUser {
            email: body.email,
            password: body.password,
            name: body.name,
            username: body.username,
            image: body.image,
            metadata_json: body.metadata_json,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await?;
    Ok(session_response(
        &state.cookie,
        &bundle,
        StatusCode::CREATED,
    ))
}

#[derive(Debug, serde::Deserialize)]
pub struct SignUpBody {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub metadata_json: Option<String>,
}

async fn sign_in_email<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Json(body): Json<SignInBody>,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    let bundle = state
        .auth
        .sign_in_email_password(SignInEmailPassword {
            email: body.email,
            password: body.password,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await?;
    Ok(session_response(&state.cookie, &bundle, StatusCode::OK))
}

#[derive(Debug, serde::Deserialize)]
pub struct SignInBody {
    pub email: String,
    pub password: String,
}

async fn session<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError>
where
    S: AuthStorage,
{
    let token = session_token_from_headers(&headers, &state.cookie)
        .ok_or(ApiError::from(AuthFlowError::InvalidCredentials))?;
    let bundle = state.auth.current_session(CurrentSession { token }).await?;
    // No token echoed back: the caller already holds it, and a GET
    // response is the most likely thing to end up in a log or a cache.
    Ok(Json(json!({
        "user": user_json(&bundle.user),
        "session": session_json(&bundle),
    })))
}

async fn refresh<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    let token = session_token_from_headers(&headers, &state.cookie)
        .ok_or(ApiError::from(AuthFlowError::InvalidCredentials))?;
    let bundle = state.auth.refresh_session(RefreshSession { token }).await?;
    Ok(session_response(&state.cookie, &bundle, StatusCode::OK))
}

async fn sign_out<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    // Idempotent by design: a caller with no token has already achieved
    // what it asked for, and saying "no such session" would leak whether
    // one existed.
    if let Some(token) = session_token_from_headers(&headers, &state.cookie) {
        state.auth.sign_out(SignOut { token }).await?;
    }
    let cleared = state.cookie.session_cookie("");
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cleared.to_string())],
    )
        .into_response())
}

async fn openapi() -> Json<Value> {
    Json(architect_auth::transport::auth_openapi_document())
}

// ── Shared shaping ───────────────────────────────────────────────────

/// A successful session response: the bundle as JSON, plus the session
/// cookie so a browser is signed in without touching the body.
fn session_response(
    cookie: &AuthCookieConfig,
    bundle: &AuthSessionBundle,
    status: StatusCode,
) -> Response {
    let set_cookie = cookie.session_cookie(bundle.token.clone());
    (
        status,
        [(header::SET_COOKIE, set_cookie.to_string())],
        Json(json!({
            "user": user_json(&bundle.user),
            "session": session_json(bundle),
            // Native clients have no cookie jar, so they read the token
            // from here and attach it as a bearer on later calls.
            "token": bundle.token,
        })),
    )
        .into_response()
}

fn user_json(user: &AuthUser) -> Value {
    json!({
        "id": user.id,
        "email": user.email,
        "name": user.name,
        "email_verified": user.email_verified,
        "image": user.image,
        "username": user.username,
        "display_username": user.display_username,
        "two_factor_enabled": user.two_factor_enabled,
        "role": user.role,
        "banned": user.banned,
        "created_at": user.created_at,
        "updated_at": user.updated_at,
    })
}

/// The session, minus `token_hash`. That field is the stored
/// verifier — serializing it would put a credential-equivalent value
/// into every response body.
fn session_json(bundle: &AuthSessionBundle) -> Value {
    json!({
        "id": bundle.session.id,
        "user_id": bundle.session.user_id,
        "expires_at": bundle.session.expires_at,
        "active_organization_id": bundle.session.active_organization_id,
        "created_at": bundle.session.created_at,
    })
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
}

/// The originating address, trusting `x-forwarded-for` because this
/// server always sits behind the cluster ingress. It is recorded on
/// sessions for the audit trail, never used for authorization.
/// Where to send a browser that reached `/oauth2/authorize` with no
/// session: the sign-in page, carrying this whole request as the place
/// to come back to.
///
/// Percent-encoded, because the authorize URL is itself full of query
/// parameters — unencoded, its first `&` would terminate `return_to`
/// and the resumed request would be missing everything after
/// `client_id`, which surfaces much later as an unrelated-looking OIDC
/// error.
fn sign_in_url(uri: &Uri) -> String {
    let original = uri.path_and_query().map_or("/", |pq| pq.as_str());
    format!("/login?return_to={}", percent_encode_query(original))
}

/// Percent-encode for use as a query-string *value*.
///
/// Hand-rolled to avoid a dependency for one call site. Unreserved
/// characters per RFC 3986 pass through; everything else, `&` `=` `?`
/// `/` `#` included, is escaped.
fn percent_encode_query(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + raw.len() / 2);
    for byte in raw.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub(crate) fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

// ── Social sign-in and account linking ───────────────────────────────

#[derive(Debug, Default, serde::Deserialize)]
pub struct SocialStartQuery {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub return_to: Option<String>,
}

/// `GET /auth/social/{provider}/start?mode=sign-in|link&return_to=…`
///
/// Mints the CSRF nonce through the engine, seals the rest of the flow
/// into the state parameter, and sends the browser to the provider.
/// `mode=link` needs a session: a browser without one is sent to sign
/// in first with this very URL as the place to come back to, so the
/// link completes after the login; a bearer caller gets `401`.
async fn social_start<S>(
    State(state): State<HttpState<S>>,
    Path(provider): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    Query(q): Query<SocialStartQuery>,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    let (provider, config) = configured_provider(&state, &provider)?;
    let mode = match q.mode.as_deref() {
        None => Mode::SignIn,
        Some(raw) => Mode::parse(raw).ok_or(ApiError::custom(
            StatusCode::BAD_REQUEST,
            "invalid_mode",
            "mode must be sign-in or link",
        ))?,
    };
    let return_to = state.social.safe_return_to(q.return_to.as_deref());

    let session_token = match mode {
        Mode::SignIn => None,
        Mode::Link => match session_token_from_headers(&headers, &state.cookie) {
            Some(token) => {
                // Fail now, not after the provider round trip, when the
                // session is dead.
                state
                    .auth
                    .current_session(CurrentSession {
                        token: token.clone(),
                    })
                    .await?;
                Some(token)
            }
            None if headers.contains_key(header::AUTHORIZATION) => {
                return Err(ApiError::from(AuthFlowError::InvalidCredentials));
            }
            None => return Ok(Redirect::to(&sign_in_url(&uri)).into_response()),
        },
    };

    let nonce = state
        .auth
        .begin_oauth_authorization(BeginOAuthAuthorization {
            provider_id: provider.id().to_owned(),
        })
        .await?;
    let flow = PendingFlow {
        mode,
        return_to,
        session_token,
    };
    let sealed = flow
        .seal(&state.auth.config.secret, &nonce.token)
        .map_err(|err| ApiError::from(AuthFlowError::Internal(err.to_string())))?;
    let url = provider.authorize_url(config, &state.social.callback_url(provider), &sealed, mode);
    Ok(Redirect::to(&url).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct SocialCallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `GET /auth/social/{provider}/callback?code&state`
///
/// Every failure after this point is a person in a browser who has just
/// been bounced back from GitHub, so failures are redirects with a short
/// `?error=` code onto the page they came from — not a JSON 4xx they
/// cannot read.
///
/// | `error` | meaning |
/// |---------|---------|
/// | `denied` | the provider reported an error (consent refused, usually) |
/// | `state` | the state was missing, tampered with, expired or reused |
/// | `exchange` | the provider refused the code |
/// | `profile` | the provider's profile could not be read |
/// | `email_in_use` | sign-in: an account with that verified email already exists — sign in and link instead |
/// | `no_email` | sign-in: the provider gave no email to create an account with |
/// | `already_linked` | link: that provider account belongs to another user |
/// | `session` | link: the session that started the link is gone |
/// | `sign_in` / `link` | any other engine refusal |
async fn social_callback<S>(
    State(state): State<HttpState<S>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    Query(q): Query<SocialCallbackQuery>,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    let (provider, config) = configured_provider(&state, &provider)?;
    let secret = state.auth.config.secret.clone();

    // The state is decoded before it is checked, so that even a failure
    // can land on the right page. Nothing is TRUSTED from it until the
    // engine has consumed the nonce below.
    let unsealed = q
        .state
        .as_deref()
        .and_then(|raw| PendingFlow::unseal(&secret, raw).ok());
    let fallback_page = match unsealed.as_ref().map(|(_, flow)| flow.mode) {
        Some(Mode::Link) => "/account",
        _ => "/login",
    };
    let return_to = unsealed
        .as_ref()
        .map(|(_, flow)| flow.return_to.clone())
        .unwrap_or_else(|| fallback_page.to_owned());

    if q.error.is_some() {
        return Ok(redirect_with(&return_to, "error", "denied"));
    }
    let Some((nonce, flow)) = unsealed else {
        return Ok(redirect_with(&return_to, "error", "state"));
    };
    if state
        .auth
        .verify_oauth_state(VerifyOAuthState {
            provider_id: provider.id().to_owned(),
            state: nonce,
        })
        .await
        .is_err()
    {
        return Ok(redirect_with(&return_to, "error", "state"));
    }
    let Some(code) = q.code.filter(|c| !c.is_empty()) else {
        return Ok(redirect_with(&return_to, "error", "exchange"));
    };

    let tokens = match state
        .social
        .client
        .exchange_code(
            provider,
            config,
            &code,
            &state.social.callback_url(provider),
        )
        .await
    {
        Ok(tokens) => tokens,
        Err(err) => {
            tracing::warn!(target: "auth_server::social", provider = provider.id(), %err, "code exchange failed");
            return Ok(redirect_with(&return_to, "error", "exchange"));
        }
    };
    let profile = match state
        .social
        .client
        .fetch_profile(provider, &tokens.access_token)
        .await
    {
        Ok(profile) => profile,
        Err(err) => {
            tracing::warn!(target: "auth_server::social", provider = provider.id(), %err, "profile fetch failed");
            return Ok(redirect_with(&return_to, "error", "profile"));
        }
    };

    match flow.mode {
        Mode::SignIn => {
            let result = state
                .auth
                .sign_in_oauth_account(SignInOAuthAccount {
                    provider_id: provider.id().to_owned(),
                    account_id: profile.account_id,
                    email: profile.email,
                    email_verified: profile.email_verified,
                    name: profile.name,
                    image: profile.image,
                    ip_address: client_ip(&headers),
                    user_agent: user_agent(&headers),
                })
                .await;
            match result {
                Ok(bundle) => Ok(redirect_signed_in(&state.cookie, &bundle, &return_to)),
                Err(AuthFlowError::InvalidInput(message)) if message.contains("email already") => {
                    Ok(redirect_with(&return_to, "error", "email_in_use"))
                }
                Err(AuthFlowError::InvalidInput(message))
                    if message.contains("email is required") =>
                {
                    Ok(redirect_with(&return_to, "error", "no_email"))
                }
                Err(err) => {
                    tracing::warn!(target: "auth_server::social", provider = provider.id(), %err, "social sign-in refused");
                    Ok(redirect_with(&return_to, "error", "sign_in"))
                }
            }
        }
        Mode::Link => {
            // The session that started the link, or — if the state did
            // not carry one — whatever the browser holds now.
            let Some(session_token) = flow
                .session_token
                .or_else(|| session_token_from_headers(&headers, &state.cookie))
            else {
                return Ok(redirect_with(&return_to, "error", "session"));
            };
            // The engine encrypts these before they touch storage; the
            // field names say "ciphertext" because that is what ends up
            // in the row.
            let result = state
                .auth
                .link_oauth_account(LinkOAuthAccount {
                    session_token,
                    provider_id: provider.id().to_owned(),
                    account_id: profile.account_id,
                    access_token_ciphertext: Some(tokens.access_token),
                    refresh_token_ciphertext: tokens.refresh_token,
                    id_token_ciphertext: tokens.id_token,
                    scope: tokens.scope.or_else(|| Some(config.scopes.join(" "))),
                })
                .await;
            match result {
                Ok(()) => Ok(redirect_with(&return_to, "linked", provider.id())),
                Err(AuthFlowError::InvalidInput(message)) if message.contains("already linked") => {
                    Ok(redirect_with(&return_to, "error", "already_linked"))
                }
                Err(AuthFlowError::InvalidCredentials | AuthFlowError::SessionExpired) => {
                    Ok(redirect_with(&return_to, "error", "session"))
                }
                Err(err) => {
                    tracing::warn!(target: "auth_server::social", provider = provider.id(), %err, "link refused");
                    Ok(redirect_with(&return_to, "error", "link"))
                }
            }
        }
    }
}

/// One linked provider account, as shown to its owner. Never a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedAccountView {
    pub provider_id: String,
    pub account_id: String,
    pub scope: Option<String>,
    pub linked_at: chrono::DateTime<chrono::Utc>,
    /// The GitHub username or the Google email, when it can be resolved.
    pub login: Option<String>,
}

impl LinkedAccountView {
    pub fn to_json(&self) -> Value {
        json!({
            "provider_id": self.provider_id,
            "account_id": self.account_id,
            "scope": self.scope,
            "linked_at": self.linked_at,
            "login": self.login,
        })
    }
}

/// `GET /auth/accounts` — the signed-in user's linked provider accounts.
async fn accounts<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError>
where
    S: AuthStorage,
{
    let token = session_token_from_headers(&headers, &state.cookie)
        .ok_or(ApiError::from(AuthFlowError::InvalidCredentials))?;
    let bundle = state.auth.current_session(CurrentSession { token }).await?;
    let accounts = linked_accounts(&state, bundle.user.id).await?;
    Ok(Json(Value::Array(
        accounts.iter().map(LinkedAccountView::to_json).collect(),
    )))
}

/// The linked accounts of a user, with a display handle where one can
/// be had without trusting anything unverified for authorization.
///
/// The row stores no username, so the handle is derived: Google's from
/// the email claim of the stored id_token; GitHub's by asking GitHub,
/// best effort — a failed lookup leaves `login` empty rather than
/// failing the page.
pub(crate) async fn linked_accounts<S>(
    state: &HttpState<S>,
    user_id: uuid::Uuid,
) -> Result<Vec<LinkedAccountView>, AuthFlowError>
where
    S: AuthStorage,
{
    let secret = &state.auth.config.secret;
    let rows = state.auth.storage.list_accounts_by_user_id(user_id).await?;
    let mut out = Vec::new();
    for row in rows {
        let Some(provider) = Provider::parse(&row.provider_id) else {
            // `credential` (the password row) and anything this server
            // does not speak.
            continue;
        };
        let login = match provider {
            Provider::Google => row
                .id_token_ciphertext
                .as_deref()
                .and_then(|ct| decrypt_secret(secret, ct).ok())
                .and_then(|id_token| email_from_id_token(&id_token)),
            Provider::GitHub => match plaintext_access_token(secret, &row) {
                Some(token) => state
                    .social
                    .client
                    .fetch_profile(provider, &token)
                    .await
                    .ok()
                    .and_then(|profile| profile.login),
                None => None,
            },
        };
        out.push(LinkedAccountView {
            provider_id: row.provider_id,
            account_id: row.account_id,
            scope: row.scope,
            linked_at: row.created_at,
            login,
        });
    }
    Ok(out)
}

fn plaintext_access_token(secret: &str, row: &AuthAccount) -> Option<String> {
    row.access_token_ciphertext
        .as_deref()
        .and_then(|ct| decrypt_secret(secret, ct).ok())
}

/// Why an unlink did not happen.
#[derive(Debug)]
pub(crate) enum UnlinkError {
    NotLinked,
    /// Removing it would leave no way to sign in.
    LastCredential,
    Auth(AuthFlowError),
}

impl From<AuthFlowError> for UnlinkError {
    fn from(error: AuthFlowError) -> Self {
        match &error {
            AuthFlowError::InvalidInput(message) if message.contains("last sign-in credential") => {
                UnlinkError::LastCredential
            }
            _ => UnlinkError::Auth(error),
        }
    }
}

impl From<UnlinkError> for ApiError {
    fn from(error: UnlinkError) -> Self {
        match error {
            UnlinkError::NotLinked => ApiError::custom(
                StatusCode::NOT_FOUND,
                "not_linked",
                "no account from that provider is linked",
            ),
            UnlinkError::LastCredential => ApiError::custom(
                StatusCode::CONFLICT,
                "last_credential",
                "unlinking this account would leave no way to sign in — set a password or link another provider first",
            ),
            UnlinkError::Auth(error) => ApiError::from(error),
        }
    }
}

/// Shared by the JSON route and the account page's form.
pub(crate) async fn unlink<S>(
    state: &HttpState<S>,
    session_token: String,
    provider: Provider,
) -> Result<(), UnlinkError>
where
    S: AuthStorage,
{
    let bundle = state
        .auth
        .current_session(CurrentSession {
            token: session_token.clone(),
        })
        .await?;
    let row = state
        .auth
        .storage
        .list_accounts_by_user_id(bundle.user.id)
        .await?
        .into_iter()
        .find(|row| row.provider_id == provider.id())
        .ok_or(UnlinkError::NotLinked)?;
    // The engine refuses to remove the last credential; that refusal is
    // what becomes the 409.
    state
        .auth
        .unlink_oauth_account(UnlinkOAuthAccount {
            session_token,
            provider_id: row.provider_id,
            account_id: row.account_id,
        })
        .await?;
    Ok(())
}

/// `POST /auth/accounts/{provider}/unlink`
async fn unlink_account<S>(
    State(state): State<HttpState<S>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError>
where
    S: AuthStorage,
{
    let provider = Provider::parse(&provider).ok_or(ApiError::custom(
        StatusCode::NOT_FOUND,
        "unknown_provider",
        "no such provider",
    ))?;
    let token = session_token_from_headers(&headers, &state.cookie)
        .ok_or(ApiError::from(AuthFlowError::InvalidCredentials))?;
    unlink(&state, token, provider).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct LinkedTokenQuery {
    #[serde(default)]
    pub provider: Option<String>,
}

/// `GET /oauth2/linked-token?provider=github`
///
/// The relying-party endpoint. The bearer is an OIDC **access token this
/// server issued** (the same credential `/oauth2/userinfo` takes), not
/// a session. It must carry the configured linked-token scope
/// (`forge:github` by default) — a client that was only granted
/// `openid email` cannot reach a person's GitHub token by accident.
///
/// Answers:
///
/// * `200 { provider, login, account_id, access_token, scope }`
/// * `401 invalid_token` — no or unverifiable bearer
/// * `403 insufficient_scope` — the token lacks the scope
/// * `404 not_linked` — the user has no such account, or it holds no token
/// * `400 unsupported_provider` — only `github` hands tokens out
///
/// `login` is fetched live from GitHub with the token, which doubles as
/// proof the token still works; if that lookup fails the token is still
/// returned and `login` is `null`.
async fn linked_token<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Query(q): Query<LinkedTokenQuery>,
) -> Result<Json<Value>, ApiError>
where
    S: AuthStorage,
{
    use architect_telemetry::wide;

    let outcome = |name: &'static str| wide::set("auth.linked_token.outcome", name);

    let provider = q.provider.as_deref().unwrap_or("github");
    if provider != Provider::GitHub.id() {
        outcome("unsupported_provider");
        return Err(ApiError::custom(
            StatusCode::BAD_REQUEST,
            "unsupported_provider",
            "only provider=github is supported",
        ));
    }
    let provider = Provider::GitHub;
    wide::set("auth.linked_token.provider", provider.id());

    let Some(access_token) = bearer(&headers) else {
        outcome("no_bearer");
        return Err(ApiError::from(AuthFlowError::InvalidCredentials));
    };
    let claims = match state
        .auth
        .verify_jwt(VerifyJwt {
            token: access_token,
            audience: None,
        })
        .await
    {
        Ok(verification) => verification.claims,
        Err(_) => {
            outcome("invalid_token");
            return Err(ApiError::custom(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "the bearer is not an access token this server issued",
            ));
        }
    };
    let required = state.social.config.linked_token_scope.as_str();
    let granted = claims
        .extra
        .as_ref()
        .and_then(|extra| extra.get("scope"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(client_id) = claims
        .extra
        .as_ref()
        .and_then(|extra| extra.get("client_id"))
        .and_then(Value::as_str)
    {
        wide::set_display("auth.linked_token.client_id", client_id);
    }
    if !granted.split_whitespace().any(|scope| scope == required) {
        outcome("insufficient_scope");
        return Err(ApiError::custom(
            StatusCode::FORBIDDEN,
            "insufficient_scope",
            "the access token was not granted the linked-token scope",
        ));
    }
    let Ok(user_id) = claims.sub.parse::<uuid::Uuid>() else {
        outcome("invalid_token");
        return Err(ApiError::from(AuthFlowError::InvalidCredentials));
    };
    wide::set_display("auth.linked_token.user_id", user_id);

    let secret = &state.auth.config.secret;
    let row = state
        .auth
        .storage
        .list_accounts_by_user_id(user_id)
        .await?
        .into_iter()
        .find(|row| row.provider_id == provider.id());
    let Some(row) = row else {
        outcome("not_linked");
        return Err(ApiError::custom(
            StatusCode::NOT_FOUND,
            "not_linked",
            "no GitHub account is linked to this user",
        ));
    };
    let Some(token) = plaintext_access_token(secret, &row) else {
        // Linked by signing in with GitHub rather than by an explicit
        // link, so no token was kept. The fix is to link (again) from
        // the account page.
        outcome("not_linked");
        return Err(ApiError::custom(
            StatusCode::NOT_FOUND,
            "not_linked",
            "the linked GitHub account holds no token; link it from the account page",
        ));
    };
    let login = state
        .social
        .client
        .fetch_profile(provider, &token)
        .await
        .ok()
        .and_then(|profile| profile.login);
    outcome("ok");
    Ok(Json(json!({
        "provider": provider.id(),
        "login": login,
        "account_id": row.account_id,
        "access_token": token,
        "scope": row.scope,
    })))
}

/// The provider named in the path, if this deployment has it on.
fn configured_provider<'a, S>(
    state: &'a HttpState<S>,
    raw: &str,
) -> Result<(Provider, &'a SocialProviderConfig), ApiError> {
    let not_found = || {
        ApiError::custom(
            StatusCode::NOT_FOUND,
            "unknown_provider",
            "that provider is not configured",
        )
    };
    let provider = Provider::parse(raw).ok_or_else(not_found)?;
    let config = state
        .social
        .provider_config(provider)
        .ok_or_else(not_found)?;
    Ok((provider, config))
}

/// `303` to `target` with one extra query parameter appended.
fn redirect_with(target: &str, key: &str, value: &str) -> Response {
    let sep = if target.contains('?') { '&' } else { '?' };
    let location = format!(
        "{target}{sep}{}={}",
        encode_component(key),
        encode_component(value)
    );
    (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
}

/// Set the session cookie and send the browser on — the social twin of
/// the login form's success path. The token also rides in a fragment
/// for nobody: a browser flow has the cookie, and a native client that
/// wants a bearer token exchanges the session over `/auth/session`.
fn redirect_signed_in(
    cookie: &AuthCookieConfig,
    bundle: &AuthSessionBundle,
    return_to: &str,
) -> Response {
    let set_cookie = cookie.session_cookie(bundle.token.clone());
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, set_cookie.to_string()),
            (header::LOCATION, return_to.to_owned()),
        ],
    )
        .into_response()
}

/// An `AuthFlowError` on its way out as HTTP.
///
/// The engine's `map_auth_error` collapses the internal taxonomy into
/// the status/code/message triple that is safe to show a caller — so an
/// internal storage failure never reaches the client as a stack trace,
/// and "no such user" and "wrong password" stay indistinguishable.
pub struct ApiError(architect_auth::transport::PublicAuthError);

impl ApiError {
    /// An error the engine's taxonomy has no entry for — a routing-level
    /// refusal such as an unconfigured provider or a missing scope.
    pub fn custom(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self(architect_auth::transport::PublicAuthError {
            status: status.as_u16(),
            code,
            message,
        })
    }
}

impl From<AuthFlowError> for ApiError {
    fn from(error: AuthFlowError) -> Self {
        Self(map_auth_error(&error))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            Json(json!({ "error": self.0.code, "message": self.0.message })),
        )
            .into_response()
    }
}
