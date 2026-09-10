//! A portable client for the auth server's session JSON API.
//!
//! `auth-client` decorates *vox* calls with a session token, which is
//! the right thing for a Rust app talking to a backend over the vox
//! WebSocket. It does not help you *get* a token in the first place,
//! and it has nothing to say to a browser build that would rather make
//! a `fetch` than open a socket.
//!
//! This crate is that half: sign-up, sign-in, session, refresh and
//! sign-out over plain HTTP+JSON, against the endpoints
//! `apps/auth-server` mounts. It compiles for native and for
//! `wasm32-unknown-unknown` (where `reqwest` goes through `fetch`), so
//! one client serves a desktop shell, a mobile app and a web SPA.
//!
//! Tokens are carried as `Authorization: Bearer`, never as cookies —
//! a bearer header sidesteps CORS credential rules and works
//! identically on every target.
//!
//! ```no_run
//! # async fn example() -> Result<(), auth_http::AuthHttpError> {
//! use auth_http::AuthHttpClient;
//!
//! let client = AuthHttpClient::new("https://auth.fasttrackstudio.app");
//! let session = client.sign_in("cody@fasttrackstudio.app", "…").await?;
//! println!("signed in as {:?}", session.user.email);
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use auth_client::{StoredSession, TokenStore};
use serde::{Deserialize, Serialize};

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
mod browser;
#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub use browser::LocalStorageTokenStore;

/// The signed-in user, as the server reports it.
///
/// Its own type rather than `auth_proto::AuthUser`: that one is a
/// `facet` entity carrying storage concerns, and this crate is
/// deliberately free of the proto/architect dependency so a wasm build
/// stays small.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub email_verified: bool,
    pub image: Option<String>,
    pub username: Option<String>,
    pub display_username: Option<String>,
    #[serde(default)]
    pub two_factor_enabled: bool,
    pub role: Option<String>,
    #[serde(default)]
    pub banned: bool,
}

/// The session, minus anything credential-shaped. The server never
/// serializes the stored token hash, so there is nothing here to leak.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthSession {
    pub id: String,
    pub user_id: String,
    pub expires_at: String,
    pub active_organization_id: Option<String>,
}

/// What a sign-in or sign-up returns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub user: AuthUser,
    pub session: AuthSession,
    /// Only present on the responses that mint a token — `GET
    /// /auth/session` deliberately does not echo it back, because the
    /// caller already holds it and a GET is the most likely thing to be
    /// logged or cached.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthHttpError {
    /// The server rejected the request and said why. `code` is from
    /// architect-auth's error taxonomy (`invalid_credentials`,
    /// `verification_required`, …) — match on it rather than on the
    /// human-readable message, which is free to change.
    #[error("{code}: {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("no session token available — sign in first")]
    NoToken,
    #[error("token store: {0}")]
    Store(#[from] auth_client::TokenStoreError),
}

impl AuthHttpError {
    /// Whether this is the server saying "you are not signed in", as
    /// opposed to a network problem. A UI should send the user to the
    /// sign-in screen for the first and retry the second.
    pub fn is_unauthenticated(&self) -> bool {
        matches!(self, Self::Api { status: 401, .. }) || matches!(self, Self::NoToken)
    }
}

/// The shape the server returns on an error.
#[derive(Deserialize)]
struct ApiErrorBody {
    error: String,
    message: String,
}

/// A client bound to one auth server.
#[derive(Clone)]
pub struct AuthHttpClient {
    base_url: String,
    http: reqwest::Client,
    store: Option<Arc<dyn TokenStore>>,
}

impl AuthHttpClient {
    /// A client for the server at `base_url` (e.g.
    /// `https://auth.fasttrackstudio.app`), holding the session only in
    /// memory.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
            store: None,
        }
    }

    /// Persist the session through `store`, so it survives a restart or
    /// a page reload. Every call that mints a token writes it here, and
    /// [`sign_out`](Self::sign_out) clears it.
    pub fn with_store(mut self, store: Arc<dyn TokenStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// The token the store currently holds, if any.
    pub fn token(&self) -> Option<String> {
        self.store
            .as_ref()
            .and_then(|store| store.load().ok().flatten())
            .map(|session| session.token)
    }

    /// Whether a token is on hand. Cheap and local — it says nothing
    /// about whether the server still considers it valid, which only
    /// [`session`](Self::session) can answer.
    pub fn has_token(&self) -> bool {
        self.token().is_some()
    }

    /// Create an account and sign in.
    pub async fn sign_up(&self, request: &SignUpRequest) -> Result<Session, AuthHttpError> {
        let response = self
            .http
            .post(self.url("/auth/sign-up/email"))
            .json(request)
            .send()
            .await?;
        let session = self.decode(response).await?;
        self.remember(&session)?;
        Ok(session)
    }

    /// Sign in with email and password.
    pub async fn sign_in(
        &self,
        email: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Session, AuthHttpError> {
        let body = SignInRequest {
            email: email.into(),
            password: password.into(),
        };
        let response = self
            .http
            .post(self.url("/auth/sign-in/email"))
            .json(&body)
            .send()
            .await?;
        let session = self.decode(response).await?;
        self.remember(&session)?;
        Ok(session)
    }

    /// Resolve the stored token to its user and session. This is the
    /// call that answers "am I still signed in?" on startup.
    pub async fn session(&self) -> Result<Session, AuthHttpError> {
        let token = self.token().ok_or(AuthHttpError::NoToken)?;
        let response = self
            .http
            .get(self.url("/auth/session"))
            .bearer_auth(token)
            .send()
            .await?;
        self.decode(response).await
    }

    /// Rotate the session: a fresh token with a new expiry, the old one
    /// deactivated.
    pub async fn refresh(&self) -> Result<Session, AuthHttpError> {
        let token = self.token().ok_or(AuthHttpError::NoToken)?;
        let response = self
            .http
            .post(self.url("/auth/refresh"))
            .bearer_auth(token)
            .send()
            .await?;
        let session = self.decode(response).await?;
        self.remember(&session)?;
        Ok(session)
    }

    /// Revoke the session and forget the token.
    ///
    /// The local token is cleared even if the network call fails —
    /// otherwise "sign me out" on a flaky connection leaves the user
    /// looking signed in, which is the worse failure.
    pub async fn sign_out(&self) -> Result<(), AuthHttpError> {
        let token = self.token();
        if let Some(store) = &self.store {
            store.clear()?;
        }
        let Some(token) = token else {
            return Ok(());
        };
        let response = self
            .http
            .post(self.url("/auth/sign-out"))
            .bearer_auth(token)
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(self.api_error(response).await)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Write the minted token to the store, if there is one.
    fn remember(&self, session: &Session) -> Result<(), AuthHttpError> {
        let (Some(store), Some(token)) = (&self.store, &session.token) else {
            return Ok(());
        };
        let mut stored = StoredSession::new(token.clone()).with_user_id(session.user.id.clone());
        if let Some(email) = &session.user.email {
            stored = stored.with_email(email.clone());
        }
        store.save(&stored)?;
        Ok(())
    }

    async fn decode(&self, response: reqwest::Response) -> Result<Session, AuthHttpError> {
        if response.status().is_success() {
            return Ok(response.json().await?);
        }
        Err(self.api_error(response).await)
    }

    /// Turn a failure response into a typed error, falling back to the
    /// status when the body is not the taxonomy shape (a proxy error
    /// page, say).
    async fn api_error(&self, response: reqwest::Response) -> AuthHttpError {
        let status = response.status().as_u16();
        match response.json::<ApiErrorBody>().await {
            Ok(body) => AuthHttpError::Api {
                status,
                code: body.error,
                message: body.message,
            },
            Err(_) => AuthHttpError::Api {
                status,
                code: "unknown".into(),
                message: format!("request failed with status {status}"),
            },
        }
    }
}

/// Sign-up fields. Only email and password are required.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SignUpRequest {
    pub email: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_json: Option<String>,
}

impl SignUpRequest {
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
            ..Default::default()
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

#[derive(Serialize)]
struct SignInRequest {
    email: String,
    password: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_trailing_slash_does_not_double_up() {
        let client = AuthHttpClient::new("https://auth.fasttrackstudio.app/");
        assert_eq!(
            client.url("/auth/session"),
            "https://auth.fasttrackstudio.app/auth/session"
        );
    }

    #[test]
    fn unauthenticated_is_distinguished_from_transport_failure() {
        let unauthorized = AuthHttpError::Api {
            status: 401,
            code: "invalid_credentials".into(),
            message: "invalid credentials".into(),
        };
        assert!(unauthorized.is_unauthenticated());
        assert!(AuthHttpError::NoToken.is_unauthenticated());

        let server_error = AuthHttpError::Api {
            status: 500,
            code: "internal".into(),
            message: "internal".into(),
        };
        assert!(!server_error.is_unauthenticated());
    }

    #[test]
    fn sign_up_omits_unset_optional_fields() {
        // The server distinguishes "absent" from "cleared", so an
        // unset name must not serialize as null.
        let json = serde_json::to_string(&SignUpRequest::new("a@b.c", "pw")).expect("serialize");
        assert_eq!(json, r#"{"email":"a@b.c","password":"pw"}"#);
    }
}
