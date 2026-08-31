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

use architect_auth::{
    ArchitectAuth, AuthStorage, AuthorizeOidc, CurrentSession, CreateEmailPasswordUser,
    ExchangeOidcToken, GetOidcUserInfo, RefreshSession, SignOut,
    transport::{AuthCookieConfig, axum::session_token_from_headers, map_auth_error},
};
use auth_proto::{AuthFlowError, AuthSessionBundle, AuthUser, SignInEmailPassword};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde_json::{Value, json};

/// Everything the HTTP handlers need: the auth engine plus the cookie
/// policy that names the session cookie.
#[derive(Clone)]
pub struct HttpState<S> {
    pub auth: ArchitectAuth<S>,
    pub cookie: AuthCookieConfig,
}

impl<S> HttpState<S> {
    pub fn new(auth: ArchitectAuth<S>, cookie: AuthCookieConfig) -> Self {
        Self { auth, cookie }
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
        .route("/oauth2/authorize", get(authorize::<S>).post(authorize::<S>))
        .route("/oauth2/token", post(token::<S>))
        .route("/oauth2/userinfo", get(userinfo::<S>).post(userinfo::<S>))
        // ── Core session JSON ────────────────────────────────────────
        .route("/auth/sign-up/email", post(sign_up_email::<S>))
        .route("/auth/sign-in/email", post(sign_in_email::<S>))
        .route("/auth/session", get(session::<S>))
        .route("/auth/refresh", post(refresh::<S>))
        .route("/auth/sign-out", post(sign_out::<S>))
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
/// Requires an already-authenticated user: the session comes from the
/// cookie (browser) or a bearer header (native app that already signed
/// in). There is no login form here — a front-end owns that, and
/// redirects back once it holds a session.
async fn authorize<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Query(params): Query<AuthorizeParams>,
) -> Result<Response, ApiError>
where
    S: AuthStorage,
{
    let session_token = session_token_from_headers(&headers, &state.cookie)
        .ok_or(ApiError::from(AuthFlowError::InvalidCredentials))?;

    let authorization = state
        .auth
        .authorize_oidc(AuthorizeOidc {
            session_token,
            client_id: params.client_id,
            redirect_uri: params.redirect_uri,
            response_type: params
                .response_type
                .unwrap_or_else(|| "code".into()),
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
    Ok(session_response(&state.cookie, &bundle, StatusCode::CREATED))
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
fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

/// An `AuthFlowError` on its way out as HTTP.
///
/// The engine's `map_auth_error` collapses the internal taxonomy into
/// the status/code/message triple that is safe to show a caller — so an
/// internal storage failure never reaches the client as a stack trace,
/// and "no such user" and "wrong password" stay indistinguishable.
pub struct ApiError(architect_auth::transport::PublicAuthError);

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
