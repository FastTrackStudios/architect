//! Upstream OAuth providers: GitHub, Google and TONE3000.
//!
//! The engine (`architect-auth`) owns the *model* — linked accounts,
//! CSRF state, the encrypted token store — but deliberately talks to no
//! provider. This module is the missing half: it knows each provider's
//! endpoints, exchanges an authorization code for tokens, and reads the
//! profile that names the account being linked. `http.rs` glues the two
//! together.
//!
//! # Trust boundaries
//!
//! * The provider client is a trait so the HTTP surface can be tested
//!   without the network — a fake hands back fixed tokens and a fixed
//!   profile, and the tests assert on what the engine stored.
//! * Tokens returned here are plaintext for exactly as long as it takes
//!   to hand them to `LinkOAuthAccount`, which encrypts them with the
//!   server secret before they reach storage.
//! * Pending-flow state travels through the provider inside the OAuth
//!   `state` parameter, encrypted and authenticated with the same secret,
//!   next to a one-time nonce the engine persisted — see
//!   [`PendingFlow`].

use std::time::Duration;

use architect_auth::crypto::{decrypt_secret, encrypt_secret};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::config::SocialProviderConfig;

/// A provider this server knows how to talk to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    GitHub,
    Google,
    /// The NAM capture library. Unlike the other two this is not a way to
    /// sign in — nobody has a `FastTrackStudio` account *because* they have a
    /// TONE3000 one — it is only ever linked to an account that exists, so
    /// the apps can browse and download captures as that person.
    Tone3000,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::GitHub, Self::Google, Self::Tone3000];

    /// Whether this provider can create or resume a session.
    ///
    /// TONE3000 cannot: its profile carries no verified email, and an
    /// identity provider is a different thing from a service you hold a
    /// token for. Letting it sign people in would mint accounts with no way
    /// to recover them.
    #[must_use]
    pub const fn can_sign_in(self) -> bool {
        !matches!(self, Self::Tone3000)
    }

    /// Whether the flow must carry PKCE.
    ///
    /// TONE3000 requires `code_challenge` on the authorization request and
    /// the matching `code_verifier` at the token endpoint; GitHub and Google
    /// are confidential clients here and authenticate with their secret.
    #[must_use]
    pub const fn uses_pkce(self) -> bool {
        matches!(self, Self::Tone3000)
    }

    /// The id used in paths, in `provider_id` on stored accounts, and in
    /// the engine's built-in provider table.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Google => "google",
            Self::Tone3000 => "tone3000",
        }
    }

    /// Human name for buttons and labels.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::Google => "Google",
            Self::Tone3000 => "TONE3000",
        }
    }

    /// The OIDC scope an access token must carry to be handed this
    /// provider's linked token.
    ///
    /// One scope per provider, so a client granted the right to act as
    /// someone on TONE3000 does not thereby get their GitHub token.
    #[must_use]
    pub const fn linked_token_scope(self) -> &'static str {
        match self {
            Self::GitHub => "forge:github",
            Self::Google => "forge:google",
            Self::Tone3000 => "tone3000",
        }
    }

    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    #[must_use]
    pub const fn authorize_endpoint(self) -> &'static str {
        match self {
            Self::GitHub => "https://github.com/login/oauth/authorize",
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::Tone3000 => "https://www.tone3000.com/api/v1/oauth/authorize",
        }
    }

    #[must_use]
    pub const fn token_endpoint(self) -> &'static str {
        match self {
            Self::GitHub => "https://github.com/login/oauth/access_token",
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::Tone3000 => "https://www.tone3000.com/api/v1/oauth/token",
        }
    }

    #[must_use]
    pub const fn userinfo_endpoint(self) -> &'static str {
        match self {
            Self::GitHub => "https://api.github.com/user",
            Self::Google => "https://openidconnect.googleapis.com/v1/userinfo",
            Self::Tone3000 => "https://www.tone3000.com/api/v1/user",
        }
    }

    /// The path this provider's endpoints sit under on a mock server.
    ///
    /// A mock serves all three, so the provider has to be in the path —
    /// `/github/authorize`, `/google/token`, and so on.
    #[must_use]
    pub const fn mock_path(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Google => "google",
            Self::Tone3000 => "tone3000",
        }
    }

    /// Where to send the browser, given an optional mock origin.
    ///
    /// `None` is the real provider. A mock origin replaces all three
    /// endpoints at once rather than one at a time: a deployment
    /// pointing *some* calls at a mock and others at the real provider
    /// would fail in ways that look like the provider misbehaving.
    #[must_use]
    pub fn authorize_endpoint_at(self, mock: Option<&str>) -> String {
        mock.map_or_else(
            || self.authorize_endpoint().to_owned(),
            |base| {
                format!(
                    "{}/{}/authorize",
                    base.trim_end_matches('/'),
                    self.mock_path()
                )
            },
        )
    }

    #[must_use]
    pub fn token_endpoint_at(self, mock: Option<&str>) -> String {
        mock.map_or_else(
            || self.token_endpoint().to_owned(),
            |base| format!("{}/{}/token", base.trim_end_matches('/'), self.mock_path()),
        )
    }

    #[must_use]
    pub fn userinfo_endpoint_at(self, mock: Option<&str>) -> String {
        mock.map_or_else(
            || self.userinfo_endpoint().to_owned(),
            |base| format!("{}/{}/user", base.trim_end_matches('/'), self.mock_path()),
        )
    }

    /// GitHub's separate address list. Only GitHub has one.
    #[must_use]
    pub fn emails_endpoint_at(mock: Option<&str>) -> String {
        mock.map_or_else(
            || "https://api.github.com/user/emails".to_owned(),
            |base| format!("{}/github/user/emails", base.trim_end_matches('/')),
        )
    }

    /// The full authorization URL the browser is sent to.
    ///
    /// Google is asked for `access_type=offline` only when linking: a
    /// refresh token is only useful when the token is going to be kept,
    /// and asking for one on plain sign-in adds a consent step for
    /// nothing. `prompt=select_account` so someone with several Google
    /// accounts is not silently signed in with whichever is active.
    #[must_use]
    pub fn authorize_url(
        self,
        config: &SocialProviderConfig,
        redirect_uri: &str,
        state: &str,
        mode: Mode,
        challenge: Option<&str>,
        mock: Option<&str>,
    ) -> String {
        let mut params: Vec<(&str, String)> = vec![
            ("client_id", config.client_id.clone()),
            ("redirect_uri", redirect_uri.to_owned()),
            ("response_type", "code".to_owned()),
            ("state", state.to_owned()),
        ];
        // TONE3000 documents no `scope` parameter, and sending an empty one
        // is not the same as sending none.
        if !config.scopes.is_empty() {
            params.push(("scope", config.scopes.join(" ")));
        }
        if let Some(challenge) = challenge {
            params.push(("code_challenge", challenge.to_owned()));
            params.push(("code_challenge_method", "S256".to_owned()));
        }
        if self == Self::Google {
            params.push(("prompt", "select_account".to_owned()));
            if mode == Mode::Link {
                params.push(("access_type", "offline".to_owned()));
            }
        }
        format!(
            "{}?{}",
            self.authorize_endpoint_at(mock),
            form_encode(&params)
        )
    }
}

/// What the browser came here to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Create or resume a session from the provider identity.
    SignIn,
    /// Attach the provider identity — and its tokens — to the signed-in
    /// user.
    Link,
}

impl Mode {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "sign-in" => Some(Self::SignIn),
            "link" => Some(Self::Link),
            _ => None,
        }
    }
}

/// What a provider hands back for a code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_in: Option<i64>,
    /// The scope the provider actually granted, which may be less than
    /// what was asked for.
    pub scope: Option<String>,
}

/// Who the token belongs to, in the shape `SignInOAuthAccount` wants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    /// The provider's stable id for the account (`id` at GitHub, `sub`
    /// at Google).
    pub account_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
    pub image: Option<String>,
    /// A human handle: the GitHub username, or the Google email.
    pub login: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("provider refused the code: {0}")]
    Exchange(String),
    #[error("provider returned an unusable profile: {0}")]
    Profile(String),
}

/// The network half, behind a trait so tests can substitute a fake.
#[async_trait::async_trait]
pub trait ProviderClient: Send + Sync {
    async fn exchange_code(
        &self,
        provider: Provider,
        config: &SocialProviderConfig,
        code: &str,
        redirect_uri: &str,
        verifier: Option<&str>,
    ) -> Result<ProviderTokens, ProviderError>;

    /// Mint a fresh access token from a stored refresh token.
    ///
    /// Only meaningful for providers whose access tokens expire — which is
    /// why it exists at all: a GitHub token is good until revoked, but a
    /// TONE3000 access token lasts an hour and its refresh token ROTATES on
    /// every use, so whoever refreshes must also persist what comes back.
    /// Doing that in one place is the reason these tokens live on the server
    /// rather than on each device.
    async fn refresh_tokens(
        &self,
        provider: Provider,
        config: &SocialProviderConfig,
        refresh_token: &str,
    ) -> Result<ProviderTokens, ProviderError>;

    async fn fetch_profile(
        &self,
        provider: Provider,
        access_token: &str,
    ) -> Result<Profile, ProviderError>;
}

/// The real thing, over reqwest.
#[derive(Clone)]
pub struct HttpProviderClient {
    http: reqwest::Client,
    /// A mock provider's origin, when one is configured. Held on the
    /// client so the token exchange and the profile fetch cannot
    /// disagree with the authorize URL about where the provider is.
    mock: Option<String>,
}

impl HttpProviderClient {
    /// Every call is bounded at 15 s: a provider that hangs must not
    /// hold a browser (and a connection) open indefinitely.
    pub fn new() -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("architect-auth-server")
            .build()?;
        Ok(Self { http, mock: None })
    }

    /// Point every provider at a mock origin instead of the real one.
    #[must_use]
    pub fn with_mock(mut self, mock: Option<String>) -> Self {
        self.mock = mock;
        self
    }
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(serde::Deserialize)]
struct GitHubUser {
    id: serde_json::Value,
    login: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

/// `GET /api/v1/user` — the account a token belongs to.
#[derive(serde::Deserialize)]
struct Tone3000User {
    /// A UUID string, and the stable identity of the account.
    id: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    avatar_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct GoogleUser {
    sub: String,
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
    name: Option<String>,
    picture: Option<String>,
}

#[async_trait::async_trait]
impl ProviderClient for HttpProviderClient {
    async fn exchange_code(
        &self,
        provider: Provider,
        config: &SocialProviderConfig,
        code: &str,
        redirect_uri: &str,
        verifier: Option<&str>,
    ) -> Result<ProviderTokens, ProviderError> {
        let mut form: Vec<(&str, &str)> = vec![
            ("client_id", config.client_id.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ];
        // A PKCE client proves itself with the verifier instead of a secret,
        // and TONE3000's token endpoint documents no `client_secret` field.
        // Sending an empty one is worse than sending none: it reads as a
        // confidential client failing to authenticate.
        if let Some(verifier) = verifier {
            form.push(("code_verifier", verifier));
        }
        if !config.client_secret.is_empty() {
            form.push(("client_secret", config.client_secret.as_str()));
        }
        // GitHub answers with form-encoding unless told otherwise; Google
        // is JSON regardless. Asking for JSON works for both.
        let response: TokenResponse = self
            .http
            .post(provider.token_endpoint_at(self.mock.as_deref()))
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&form)
            .send()
            .await?
            .json()
            .await?;
        if let Some(error) = response.error {
            return Err(ProviderError::Exchange(format!(
                "{error}: {}",
                response.error_description.unwrap_or_default()
            )));
        }
        let access_token = response
            .access_token
            .ok_or_else(|| ProviderError::Exchange("no access_token in response".into()))?;
        Ok(ProviderTokens {
            access_token,
            refresh_token: response.refresh_token,
            id_token: response.id_token,
            expires_in: response.expires_in,
            scope: response.scope,
        })
    }

    async fn refresh_tokens(
        &self,
        provider: Provider,
        config: &SocialProviderConfig,
        refresh_token: &str,
    ) -> Result<ProviderTokens, ProviderError> {
        let mut form: Vec<(&str, &str)> = vec![
            ("client_id", config.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ];
        if !config.client_secret.is_empty() {
            form.push(("client_secret", config.client_secret.as_str()));
        }
        let response: TokenResponse = self
            .http
            .post(provider.token_endpoint())
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&form)
            .send()
            .await?
            .json()
            .await?;
        if let Some(error) = response.error {
            return Err(ProviderError::Exchange(format!(
                "{error}: {}",
                response.error_description.unwrap_or_default()
            )));
        }
        let access_token = response
            .access_token
            .ok_or_else(|| ProviderError::Exchange("no access_token in response".into()))?;
        Ok(ProviderTokens {
            access_token,
            refresh_token: response.refresh_token,
            id_token: response.id_token,
            expires_in: response.expires_in,
            scope: response.scope,
        })
    }

    async fn fetch_profile(
        &self,
        provider: Provider,
        access_token: &str,
    ) -> Result<Profile, ProviderError> {
        match provider {
            Provider::GitHub => {
                let user: GitHubUser = self
                    .http
                    .get(provider.userinfo_endpoint_at(self.mock.as_deref()))
                    .bearer_auth(access_token)
                    .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                // A GitHub profile email is whatever the person chose to
                // make public, and often nothing. The emails endpoint
                // (needs `user:email`) says which address is primary and
                // verified — the only one worth trusting for matching.
                let (email, email_verified) = if let Some(email) = user.email {
                    (Some(email), false)
                } else {
                    let emails: Vec<GitHubEmail> = self
                        .http
                        .get(Provider::emails_endpoint_at(self.mock.as_deref()))
                        .bearer_auth(access_token)
                        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                        .send()
                        .await?
                        .error_for_status()?
                        .json()
                        .await
                        .unwrap_or_default();
                    let primary = emails
                        .iter()
                        .find(|e| e.primary && e.verified)
                        .or_else(|| emails.iter().find(|e| e.verified));
                    primary.map_or((None, false), |e| (Some(e.email.clone()), true))
                };
                let account_id = match &user.id {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    other => {
                        return Err(ProviderError::Profile(format!("unexpected id {other}")));
                    }
                };
                Ok(Profile {
                    account_id,
                    email,
                    email_verified,
                    name: user.name,
                    image: user.avatar_url,
                    login: Some(user.login),
                })
            }
            Provider::Google => {
                let user: GoogleUser = self
                    .http
                    .get(provider.userinfo_endpoint_at(self.mock.as_deref()))
                    .bearer_auth(access_token)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if user.sub.is_empty() {
                    return Err(ProviderError::Profile("empty sub".into()));
                }
                Ok(Profile {
                    account_id: user.sub,
                    login: user.email.clone(),
                    email: user.email,
                    email_verified: user.email_verified,
                    name: user.name,
                    image: user.picture,
                })
            }
            Provider::Tone3000 => {
                let user: Tone3000User = self
                    .http
                    .get(provider.userinfo_endpoint_at(self.mock.as_deref()))
                    .bearer_auth(access_token)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if user.id.is_empty() {
                    return Err(ProviderError::Profile("empty id".into()));
                }
                // No email, and none is wanted: TONE3000 is a service this
                // account holds a token for, not a way to become one. An
                // email here would be an identity claim we cannot verify.
                Ok(Profile {
                    account_id: user.id,
                    login: (!user.username.is_empty()).then_some(user.username.clone()),
                    email: None,
                    email_verified: false,
                    name: (!user.username.is_empty()).then_some(user.username),
                    image: user.avatar_url,
                })
            }
        }
    }
}

// ── Pending-flow state ───────────────────────────────────────────────

/// What has to survive the round trip through the provider.
///
/// The engine's `BeginOAuthAuthorization` persists a one-time nonce and
/// nothing else, so the rest rides in the `state` parameter itself:
/// `<nonce>.<sealed payload>`. The payload is AEAD-encrypted with the
/// server secret (the engine's own secret-envelope format), so it is
/// neither readable nor forgeable by the browser or the provider, and
/// the nonce is consumed by `VerifyOAuthState` so a state cannot be
/// replayed. Nothing else needs a table, and a pod restart between
/// `start` and `callback` loses nothing.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingFlow {
    pub mode: Mode,
    /// Already validated by the caller; used verbatim on the way out.
    pub return_to: String,
    /// The session being linked to. Only for [`Mode::Link`]; carried so
    /// a link started with a bearer token (a native app) completes even
    /// though the callback arrives from a browser with no cookie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
    /// The PKCE verifier, for providers that require it.
    ///
    /// It travels here rather than in a server-side table because this
    /// whole struct is already encrypted and authenticated with the server
    /// secret before it becomes the OAuth `state`, and it is already the
    /// thing the callback must present to prove it belongs to the request
    /// that started. A verifier needs exactly those properties and nothing
    /// more: it is single-use, short-lived, and meaningless without the
    /// code it is paired with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<String>,
}

/// A PKCE verifier and its S256 challenge.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// Mint a PKCE pair.
///
/// The verifier is the engine's own token generator — 32 bytes of OS
/// randomness, base64url — which is exactly what RFC 7636 asks for.
pub fn generate_pkce() -> Result<Pkce, architect_auth::crypto::TokenError> {
    use sha2::{Digest, Sha256};
    let verifier = architect_auth::crypto::generate_token()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(Pkce {
        verifier,
        challenge,
    })
}

impl PendingFlow {
    /// Seal into the state parameter next to the engine's nonce.
    pub fn seal(&self, secret: &str, nonce: &str) -> Result<String, StateError> {
        let json = serde_json::to_string(self).map_err(|_| StateError::Malformed)?;
        let envelope = encrypt_secret(secret, &json).map_err(|_| StateError::Seal)?;
        // The envelope is `v2.key.nonce.ct` — dots inside are fine because
        // the split on the way back is at the FIRST dot only, and the
        // engine's nonce is base64url with no dots in it.
        Ok(format!("{nonce}.{}", URL_SAFE_NO_PAD.encode(envelope)))
    }

    /// Split a state parameter into the nonce the engine checks and the
    /// payload it carried. Does not itself consume the nonce.
    pub fn unseal(secret: &str, state: &str) -> Result<(String, Self), StateError> {
        let (nonce, sealed) = state.split_once('.').ok_or(StateError::Malformed)?;
        if nonce.is_empty() || sealed.is_empty() {
            return Err(StateError::Malformed);
        }
        let envelope = URL_SAFE_NO_PAD
            .decode(sealed)
            .map_err(|_| StateError::Malformed)?;
        let envelope = String::from_utf8(envelope).map_err(|_| StateError::Malformed)?;
        let json = decrypt_secret(secret, &envelope).map_err(|_| StateError::Tampered)?;
        let flow = serde_json::from_str(&json).map_err(|_| StateError::Malformed)?;
        Ok((nonce.to_owned(), flow))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StateError {
    #[error("state is not in the expected shape")]
    Malformed,
    #[error("state could not be sealed")]
    Seal,
    #[error("state did not authenticate")]
    Tampered,
}

// ── Small helpers ────────────────────────────────────────────────────

/// `application/x-www-form-urlencoded` for a query string. Hand-rolled
/// to keep the dependency list where it is; spaces become `%20` (not
/// `+`), which every provider accepts in a query.
#[must_use]
pub fn form_encode(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encode a value going into an OAuth query string.
#[must_use]
pub fn encode_component(raw: &str) -> String {
    architect_auth::percent::encode_component(raw)
}

/// Read the `email` claim out of an `id_token` we stored ourselves.
///
/// Unverified on purpose: this is our own copy of a token the provider
/// handed us over TLS at link time, kept encrypted since. It is used
/// only for a display label, never for authorization.
pub fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "a-secret-at-least-32-bytes-long!!";

    fn github() -> SocialProviderConfig {
        SocialProviderConfig {
            client_id: "gh-client".into(),
            client_secret: "gh-secret".into(),
            scopes: vec!["repo".into(), "read:user".into(), "user:email".into()],
        }
    }

    #[test]
    fn authorize_url_carries_every_required_parameter() {
        let url = Provider::GitHub.authorize_url(
            &github(),
            "https://auth.example.com/auth/social/github/callback",
            "the-state",
            Mode::SignIn,
            None,
            None,
        );
        assert!(url.starts_with("https://github.com/login/oauth/authorize?"));
        assert!(url.contains("client_id=gh-client"));
        assert!(url.contains(
            "redirect_uri=https%3A%2F%2Fauth.example.com%2Fauth%2Fsocial%2Fgithub%2Fcallback"
        ));
        assert!(url.contains("scope=repo%20read%3Auser%20user%3Aemail"));
        assert!(url.contains("state=the-state"));
        // GitHub gets no Google-only parameters.
        assert!(!url.contains("access_type"));
    }

    #[test]
    fn google_asks_for_offline_access_only_when_linking() {
        let config = SocialProviderConfig {
            client_id: "g".into(),
            client_secret: "s".into(),
            scopes: vec!["openid".into(), "email".into(), "profile".into()],
        };
        let sign_in =
            Provider::Google.authorize_url(&config, "https://x/cb", "s", Mode::SignIn, None, None);
        let link =
            Provider::Google.authorize_url(&config, "https://x/cb", "s", Mode::Link, None, None);
        assert!(sign_in.contains("prompt=select_account"));
        assert!(!sign_in.contains("access_type=offline"));
        assert!(link.contains("access_type=offline"));
    }

    /// TONE3000 requires PKCE, and documents no `scope` parameter — an
    /// empty one is not the same as none.
    #[test]
    fn tone3000_carries_pkce_and_no_scope() {
        let config = SocialProviderConfig {
            client_id: "t3k_pub_abc".into(),
            client_secret: String::new(),
            scopes: Vec::new(),
        };
        let url = Provider::Tone3000.authorize_url(
            &config,
            "https://auth.example.com/auth/social/tone3000/callback",
            "the-state",
            Mode::Link,
            Some("the-challenge"),
            None,
        );
        assert!(url.starts_with("https://www.tone3000.com/api/v1/oauth/authorize?"));
        assert!(url.contains("client_id=t3k_pub_abc"));
        assert!(url.contains("code_challenge=the-challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=the-state"));
        assert!(!url.contains("scope="), "no scope parameter at all: {url}");
        assert!(!url.contains("client_secret"));
    }

    /// A PKCE pair must actually verify — a challenge that is not the
    /// SHA-256 of its verifier fails at the token endpoint, on someone
    /// else's machine, with a message that does not say why.
    #[test]
    fn a_generated_pkce_pair_verifies() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let pkce = generate_pkce().expect("pkce");
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        assert!(pkce.verifier.len() >= 43, "RFC 7636 wants 43..=128 chars");
    }

    /// TONE3000 is a service you hold a token for, not a way to become an
    /// account: it has no verified email to build one from.
    #[test]
    fn tone3000_is_link_only() {
        assert!(!Provider::Tone3000.can_sign_in());
        assert!(Provider::GitHub.can_sign_in());
        assert!(Provider::Google.can_sign_in());
    }

    /// One scope per provider: being trusted to act as someone on TONE3000
    /// must not also hand over their GitHub token.
    #[test]
    fn linked_token_scopes_are_not_shared_between_providers() {
        let scopes = [
            Provider::GitHub.linked_token_scope(),
            Provider::Google.linked_token_scope(),
            Provider::Tone3000.linked_token_scope(),
        ];
        let unique: std::collections::HashSet<_> = scopes.iter().collect();
        assert_eq!(unique.len(), scopes.len(), "{scopes:?}");
    }

    #[test]
    fn a_sealed_flow_carries_the_verifier_back() {
        let flow = PendingFlow {
            mode: Mode::Link,
            return_to: "/account".into(),
            session_token: None,
            verifier: Some("the-verifier".into()),
        };
        let sealed = flow
            .seal("a-server-secret-that-is-long-enough", "nonce")
            .unwrap();
        assert!(
            !sealed.contains("the-verifier"),
            "the verifier must not be readable in the state parameter"
        );
        let (nonce, back) =
            PendingFlow::unseal("a-server-secret-that-is-long-enough", &sealed).expect("unseals");
        assert_eq!(nonce, "nonce");
        assert_eq!(back.verifier.as_deref(), Some("the-verifier"));
    }

    /// The state parameter is the only thing that connects the callback
    /// to what the person set out to do, and it passes through a third
    /// party. It must come back intact, and must not come back altered.
    #[test]
    fn pending_flow_seals_and_unseals_and_detects_tampering() {
        let flow = PendingFlow {
            mode: Mode::Link,
            return_to: "/account?tab=linked".into(),
            session_token: Some("session-token-value".into()),
            verifier: None,
        };
        let state = flow.seal(SECRET, "nonce-abc").expect("seal");
        assert!(state.starts_with("nonce-abc."));
        // No plaintext leaks into the URL.
        assert!(!state.contains("session-token-value"));
        assert!(!state.contains("account"));

        let (nonce, back) = PendingFlow::unseal(SECRET, &state).expect("unseal");
        assert_eq!(nonce, "nonce-abc");
        assert_eq!(back, flow);

        // Another server's secret cannot read it.
        assert_eq!(
            PendingFlow::unseal("another-secret-at-least-32-bytes!!", &state).unwrap_err(),
            StateError::Tampered
        );
        // A flipped byte in the payload is refused, not misread.
        let mut bytes = state.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(PendingFlow::unseal(SECRET, &tampered).is_err());
        // Garbage is malformed, not a panic.
        assert_eq!(
            PendingFlow::unseal(SECRET, "no-dot").unwrap_err(),
            StateError::Malformed
        );
    }

    #[test]
    fn email_is_read_from_an_id_token_payload() {
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"1","email":"me@example.com"}"#);
        let token = format!("eyJhbGciOiJSUzI1NiJ9.{payload}.sig");
        assert_eq!(
            email_from_id_token(&token),
            Some("me@example.com".to_owned())
        );
        assert_eq!(email_from_id_token("not-a-jwt"), None);
    }

    #[test]
    fn provider_ids_round_trip() {
        for provider in Provider::ALL {
            assert_eq!(Provider::parse(provider.id()), Some(provider));
        }
        assert_eq!(Provider::parse("facebook"), None);
    }
}
