//! Upstream OAuth providers: GitHub and Google.
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
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::GitHub, Provider::Google];

    /// The id used in paths, in `provider_id` on stored accounts, and in
    /// the engine's built-in provider table.
    pub fn id(self) -> &'static str {
        match self {
            Provider::GitHub => "github",
            Provider::Google => "google",
        }
    }

    /// Human name for buttons and labels.
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::GitHub => "GitHub",
            Provider::Google => "Google",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn authorize_endpoint(self) -> &'static str {
        match self {
            Provider::GitHub => "https://github.com/login/oauth/authorize",
            Provider::Google => "https://accounts.google.com/o/oauth2/v2/auth",
        }
    }

    pub fn token_endpoint(self) -> &'static str {
        match self {
            Provider::GitHub => "https://github.com/login/oauth/access_token",
            Provider::Google => "https://oauth2.googleapis.com/token",
        }
    }

    pub fn userinfo_endpoint(self) -> &'static str {
        match self {
            Provider::GitHub => "https://api.github.com/user",
            Provider::Google => "https://openidconnect.googleapis.com/v1/userinfo",
        }
    }

    /// The full authorization URL the browser is sent to.
    ///
    /// Google is asked for `access_type=offline` only when linking: a
    /// refresh token is only useful when the token is going to be kept,
    /// and asking for one on plain sign-in adds a consent step for
    /// nothing. `prompt=select_account` so someone with several Google
    /// accounts is not silently signed in with whichever is active.
    pub fn authorize_url(
        self,
        config: &SocialProviderConfig,
        redirect_uri: &str,
        state: &str,
        mode: Mode,
    ) -> String {
        let mut params: Vec<(&str, String)> = vec![
            ("client_id", config.client_id.clone()),
            ("redirect_uri", redirect_uri.to_owned()),
            ("response_type", "code".to_owned()),
            ("scope", config.scopes.join(" ")),
            ("state", state.to_owned()),
        ];
        if self == Provider::Google {
            params.push(("prompt", "select_account".to_owned()));
            if mode == Mode::Link {
                params.push(("access_type", "offline".to_owned()));
            }
        }
        format!("{}?{}", self.authorize_endpoint(), form_encode(&params))
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
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "sign-in" => Some(Mode::SignIn),
            "link" => Some(Mode::Link),
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
}

impl HttpProviderClient {
    /// Every call is bounded at 15 s: a provider that hangs must not
    /// hold a browser (and a connection) open indefinitely.
    pub fn new() -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("architect-auth-server")
            .build()?;
        Ok(Self { http })
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
    ) -> Result<ProviderTokens, ProviderError> {
        let form = [
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ];
        // GitHub answers with form-encoding unless told otherwise; Google
        // is JSON regardless. Asking for JSON works for both.
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
                    .get(provider.userinfo_endpoint())
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
                let (email, email_verified) = match user.email {
                    Some(email) => (Some(email), false),
                    None => {
                        let emails: Vec<GitHubEmail> = self
                            .http
                            .get("https://api.github.com/user/emails")
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
                        match primary {
                            Some(e) => (Some(e.email.clone()), true),
                            None => (None, false),
                        }
                    }
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
                    .get(provider.userinfo_endpoint())
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
pub fn form_encode(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + raw.len() / 2);
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Read the `email` claim out of an id_token we stored ourselves.
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
        let sign_in = Provider::Google.authorize_url(&config, "https://x/cb", "s", Mode::SignIn);
        let link = Provider::Google.authorize_url(&config, "https://x/cb", "s", Mode::Link);
        assert!(sign_in.contains("prompt=select_account"));
        assert!(!sign_in.contains("access_type=offline"));
        assert!(link.contains("access_type=offline"));
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
        let mut bytes = state.clone().into_bytes();
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
