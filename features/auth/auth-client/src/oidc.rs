//! The redirect sign-in, minus the plumbing.
//!
//! Every `FastTrackStudio` app — Task, Session, Signal, Keyflow, Ignition
//! — signs people in the same way: send the browser to the issuer, get
//! an authorization code back, redeem it for a token. Five apps doing
//! that is five chances to build the challenge wrong, and the failure
//! mode is unusually cruel: the issuer answers a bad PKCE exchange with
//! a generic refusal that names no field, so an app with a subtly wrong
//! base64 alphabet looks exactly like an app with a wrong client id.
//!
//! So the parts that are easy to get wrong and impossible to debug live
//! here, once, with tests.
//!
//! # What this module does not do
//!
//! No HTTP, no browser, no randomness. That is not squeamishness about
//! dependencies — it is what keeps this crate wasm-clean and tiny, which
//! its own header calls out as deliberate, and it means the same code is
//! testable on a native target with no browser to stub.
//!
//! An app supplies:
//!
//! * **entropy** — [`Pkce::from_entropy`] takes the bytes rather than
//!   generating them, because the right source is platform-specific
//!   (`crypto.getRandomValues` in a browser, `getrandom` natively) and
//!   the wrong one silently defeats PKCE. Making it an argument means an
//!   app cannot reach for `Math.random` without noticing.
//! * **the two requests** — build them with [`authorize_url`] and
//!   [`token_request_body`], send them however the app already sends
//!   things, then read the answers with [`token_response_from`] (or
//!   [`access_token_from`] when only the access token matters) and
//!   [`user_from`].
//! * **somewhere to park the verifier** while the browser is away.
//!   `sessionStorage` is the right answer on web: the verifier is
//!   meaningful for one attempt in one tab, and `localStorage` would
//!   leave it readable by every later page load in the origin.
//!
//! # Staying signed in
//!
//! An access token lasts about an hour. A sign-in that asked for
//! `offline_access` also gets a refresh token, which
//! [`TokenResponse::into_session`] keeps in the [`StoredSession`]. Before
//! a call, [`refresh_if_needed`] trades it for a new pair when the access
//! token is about to expire, and saves the result; after a 401,
//! [`refresh_now`] does the same unconditionally. Both take the app's
//! HTTP as a closure, like everything else here:
//!
//! ```ignore
//! let now = unix_now();
//! let refreshed = oidc::refresh_if_needed(&*store, "signal", now, |body| async move {
//!     http.post(format!("{issuer}{}", oidc::TOKEN_PATH))
//!         .header("content-type", "application/x-www-form-urlencoded")
//!         .body(body)
//!         .send()
//!         .await?
//!         .text()
//!         .await
//! })
//! .await?;
//! ```
//!
//! The issuer rotates refresh tokens: each one redeems once. Two
//! refreshes racing on the same store means the loser presents a spent
//! token and is refused, so an app runs at most one refresh per store at
//! a time.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::{StoredSession, TokenStore, TokenStoreError};

/// Where codes and refresh tokens are redeemed, relative to the issuer.
pub const TOKEN_PATH: &str = "/oauth2/token";

/// How long before its expiry [`refresh_if_needed`] renews a token.
///
/// A minute covers clock skew between the device and the issuer and the
/// time the call that follows takes to arrive.
pub const REFRESH_MARGIN_SECS: i64 = 60;

/// What went wrong reading the issuer's answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OidcError {
    #[error("the issuer's response was not JSON: {0}")]
    Malformed(String),
    #[error("no {0} in the issuer's response")]
    Missing(&'static str),
    /// The `state` coming back is not the one that went out.
    ///
    /// Either a stale tab finishing an abandoned attempt or a forged
    /// callback. Both are refused: the difference is not knowable from
    /// here, and treating a forgery as staleness is the dangerous half.
    #[error("sign-in state did not match")]
    StateMismatch,
    /// The issuer answered with an OAuth error (RFC 6749 §5.2), e.g.
    /// `invalid_grant` for a spent or revoked refresh token. For a
    /// refresh, that means signing in again.
    #[error("the issuer refused: {error}")]
    Refused {
        /// The OAuth error code (`invalid_grant`, `invalid_client`, …).
        error: String,
        /// The issuer's `error_description`, if it sent one.
        description: Option<String>,
    },
}

/// One sign-in attempt's secrets.
///
/// Created before leaving for the issuer, consumed on the way back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    verifier: String,
    challenge: String,
    state: String,
}

impl Pkce {
    /// Build from caller-supplied entropy.
    ///
    /// 32 bytes of verifier is the RFC 7636 floor once base64url'd (43
    /// characters) and exactly the 256 bits a `S256` challenge hashes.
    ///
    /// The bytes must come from a cryptographically secure source. A
    /// guessable verifier is not a weaker PKCE, it is no PKCE: the whole
    /// mechanism is "only the app that asked for this code knows the
    /// value behind the challenge".
    #[must_use]
    pub fn from_entropy(verifier_bytes: [u8; 32], state_bytes: [u8; 16]) -> Self {
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
        Self {
            challenge: challenge_for(&verifier),
            state: URL_SAFE_NO_PAD.encode(state_bytes),
            verifier,
        }
    }

    /// Reconstruct from a parked verifier and state, on the way back.
    #[must_use]
    pub fn resume(verifier: impl Into<String>, state: impl Into<String>) -> Self {
        let verifier = verifier.into();
        Self {
            challenge: challenge_for(&verifier),
            verifier,
            state: state.into(),
        }
    }

    /// Park this before navigating away — a verifier that did not reach
    /// storage first is a sign-in that can never complete.
    #[must_use]
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    #[must_use]
    pub fn challenge(&self) -> &str {
        &self.challenge
    }

    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Check the `state` that came back against the one that went out.
    ///
    /// # Errors
    /// [`OidcError::StateMismatch`] if they differ.
    pub fn check_state(&self, returned: &str) -> Result<(), OidcError> {
        if self.state == returned {
            Ok(())
        } else {
            Err(OidcError::StateMismatch)
        }
    }
}

/// `base64url(sha256(verifier))` — the `S256` challenge.
///
/// The base64 *url* alphabet with no padding, over the ASCII bytes of
/// the verifier. Each of those three is a way to be wrong that the
/// issuer reports identically.
#[must_use]
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The URL that starts a sign-in.
#[must_use]
pub fn authorize_url(
    issuer: &str,
    client_id: &str,
    redirect_uri: &str,
    pkce: &Pkce,
    scope: &str,
) -> String {
    format!(
        "{}/oauth2/authorize\
         ?client_id={}\
         &redirect_uri={}\
         &response_type=code\
         &scope={}\
         &code_challenge={}\
         &code_challenge_method=S256\
         &state={}",
        issuer.trim_end_matches('/'),
        encode(client_id),
        encode(redirect_uri),
        encode(scope),
        encode(pkce.challenge()),
        encode(pkce.state()),
    )
}

/// The scope every app asks for unless it needs something else.
pub const DEFAULT_SCOPE: &str = "openid email profile";

/// The form-encoded body that redeems a code.
///
/// `content-type: application/x-www-form-urlencoded`.
#[must_use]
pub fn token_request_body(client_id: &str, redirect_uri: &str, code: &str, pkce: &Pkce) -> String {
    format!(
        "grant_type=authorization_code\
         &code={}\
         &redirect_uri={}\
         &client_id={}\
         &code_verifier={}",
        encode(code),
        encode(redirect_uri),
        encode(client_id),
        encode(pkce.verifier()),
    )
}

/// Percent-encode a query-string value.
///
/// Unreserved characters per RFC 3986 pass through; everything else is
/// escaped. It matters most for `redirect_uri`, whose `:` and `/` would
/// otherwise read as structure in the URL being built.
fn encode(raw: &str) -> String {
    use std::fmt::Write as _;

    // Worst case every byte escapes to three characters.
    let mut out = String::with_capacity(raw.len().saturating_mul(3));
    for byte in raw.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*byte));
            }
            // Writing to a `String` is infallible.
            other => {
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// The form-encoded body that trades a refresh token for a new access
/// token: `grant_type=refresh_token`, as a public client (no secret).
///
/// Post it to [`TOKEN_PATH`] with
/// `content-type: application/x-www-form-urlencoded`, and read the answer
/// with [`token_response_from`].
#[must_use]
pub fn refresh_request_body(client_id: &str, refresh_token: &str) -> String {
    format!(
        "grant_type=refresh_token\
         &refresh_token={}\
         &client_id={}",
        encode(refresh_token),
        encode(client_id),
    )
}

/// A `/oauth2/token` answer, for a code redemption or a refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenResponse {
    /// The bearer token to present on calls. Never empty.
    pub access_token: String,
    /// Present when the sign-in asked for `offline_access`. Rotated on
    /// every refresh: keep the newest one.
    pub refresh_token: Option<String>,
    /// Seconds until `access_token` expires, from when it was issued.
    pub expires_in: Option<u64>,
    /// Normally `Bearer`.
    pub token_type: Option<String>,
    /// The scopes granted, space separated.
    pub scope: Option<String>,
    /// The OIDC ID token, on a code redemption that asked for `openid`.
    pub id_token: Option<String>,
}

impl TokenResponse {
    /// When `access_token` expires, given the unix time the answer
    /// arrived. `None` when the issuer did not say.
    #[must_use]
    pub fn expires_at_unix(&self, now_unix: i64) -> Option<i64> {
        let secs = i64::try_from(self.expires_in?).unwrap_or(i64::MAX);
        Some(now_unix.saturating_add(secs))
    }

    /// A session to save after signing in: the access token, its expiry
    /// and the refresh token. Add the user with
    /// [`StoredSession::with_user_id`] / [`StoredSession::with_email`]
    /// from [`user_from`].
    #[must_use]
    pub fn into_session(self, now_unix: i64) -> StoredSession {
        let expires_at_unix = self.expires_at_unix(now_unix);
        StoredSession {
            token: self.access_token,
            user_id: None,
            email: None,
            expires_at_unix,
            refresh_token: self.refresh_token,
        }
    }
}

/// Parse a `/oauth2/token` response.
///
/// Blank strings count as absent, and `expires_in` is accepted as a
/// number or a numeric string (some issuers send the latter).
///
/// # Errors
/// [`OidcError::Malformed`] if it is not JSON, [`OidcError::Refused`] if
/// it is an OAuth error answer, [`OidcError::Missing`] if there is no
/// usable `access_token` — including one present but empty, which would
/// otherwise pass for a signed-in session.
pub fn token_response_from(body: &str) -> Result<TokenResponse, OidcError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| OidcError::Malformed(e.to_string()))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(std::borrow::ToOwned::to_owned)
    };
    let Some(access_token) = text("access_token") else {
        return Err(
            text("error").map_or(OidcError::Missing("access_token"), |error| {
                OidcError::Refused {
                    error,
                    description: text("error_description"),
                }
            }),
        );
    };
    let expires_in = value.get("expires_in").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    });
    Ok(TokenResponse {
        access_token,
        refresh_token: text("refresh_token"),
        expires_in,
        token_type: text("token_type"),
        scope: text("scope"),
        id_token: text("id_token"),
    })
}

/// Read the access token out of a `/oauth2/token` response.
///
/// The access token alone; [`token_response_from`] has the rest, which a
/// native app that wants to stay signed in needs.
///
/// # Errors
/// As [`token_response_from`].
pub fn access_token_from(body: &str) -> Result<String, OidcError> {
    token_response_from(body).map(|response| response.access_token)
}

/// What [`refresh_if_needed`] / [`refresh_now`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// Nothing is stored: there is no session to keep alive.
    SignedOut,
    /// The stored token is not close to expiry (or its expiry is
    /// unknown); nothing was sent.
    Fresh(StoredSession),
    /// The token is due but there is no refresh token to renew it with:
    /// the session ends at its expiry, and only signing in again helps.
    NoRefreshToken(StoredSession),
    /// Renewed, and the new session saved.
    Refreshed(StoredSession),
}

/// Why a refresh did not complete.
///
/// `E` is the app's own transport error, from its `send` closure.
#[derive(Debug, thiserror::Error)]
pub enum RefreshError<E> {
    /// Loading or saving the session failed.
    #[error(transparent)]
    Store(#[from] TokenStoreError),
    /// Posting the refresh failed before the issuer answered.
    #[error("the refresh request failed: {0}")]
    Send(E),
    /// The issuer's answer was a refusal or unreadable. A refusal
    /// ([`OidcError::Refused`], typically `invalid_grant`) means the
    /// refresh token is spent or revoked: sign in again. The stored
    /// session is left as it was.
    #[error(transparent)]
    Oidc(#[from] OidcError),
}

/// Renew the stored session if its access token expires within
/// [`REFRESH_MARGIN_SECS`] of `now_unix`, and save the rotated session.
///
/// `send` posts the form body it is given to the issuer's [`TOKEN_PATH`]
/// and returns the response body, whatever the status: an error answer
/// is read as one. See the module docs for an example.
///
/// Returns without sending anything when nothing is stored, when the
/// token is not due, and when there is no refresh token.
#[allow(clippy::future_not_send)] // as `Send` as the caller's `send`; wasm fetch futures are not
pub async fn refresh_if_needed<S, F, Fut, E>(
    store: &S,
    client_id: &str,
    now_unix: i64,
    send: F,
) -> Result<Refresh, RefreshError<E>>
where
    S: TokenStore + ?Sized,
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, E>>,
{
    let Some(session) = store.load()? else {
        return Ok(Refresh::SignedOut);
    };
    if !session.expires_within(now_unix, REFRESH_MARGIN_SECS) {
        return Ok(Refresh::Fresh(session));
    }
    rotate(store, session, client_id, now_unix, send).await
}

/// Renew the stored session now, whatever its expiry says, and save the
/// rotated session — the retry after a call came back 401.
///
/// Same `send` contract as [`refresh_if_needed`].
#[allow(clippy::future_not_send)] // as `Send` as the caller's `send`; wasm fetch futures are not
pub async fn refresh_now<S, F, Fut, E>(
    store: &S,
    client_id: &str,
    now_unix: i64,
    send: F,
) -> Result<Refresh, RefreshError<E>>
where
    S: TokenStore + ?Sized,
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, E>>,
{
    let Some(session) = store.load()? else {
        return Ok(Refresh::SignedOut);
    };
    rotate(store, session, client_id, now_unix, send).await
}

#[allow(clippy::future_not_send)] // as `Send` as the caller's `send`
async fn rotate<S, F, Fut, E>(
    store: &S,
    session: StoredSession,
    client_id: &str,
    now_unix: i64,
    send: F,
) -> Result<Refresh, RefreshError<E>>
where
    S: TokenStore + ?Sized,
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, E>>,
{
    let Some(body) = session
        .refresh_token
        .as_deref()
        .map(|refresh_token| refresh_request_body(client_id, refresh_token))
    else {
        return Ok(Refresh::NoRefreshToken(session));
    };
    let answer = send(body).await.map_err(RefreshError::Send)?;
    let rotated = session.rotated(token_response_from(&answer)?, now_unix);
    store.save(&rotated)?;
    Ok(Refresh::Refreshed(rotated))
}

/// Who a token belongs to, per `/oauth2/userinfo`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UserInfo {
    /// The issuer's user id. This is the principal that membership rows
    /// are keyed on.
    pub sub: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// Parse a `/oauth2/userinfo` response.
///
/// `sub` is the only claim required. Email and name are absent for an
/// account that has not set them, and a sign-in has to survive that
/// rather than failing on a missing display name.
///
/// # Errors
/// [`OidcError::Malformed`] if it is not JSON, [`OidcError::Missing`] if
/// there is no `sub`.
pub fn user_from(body: &str) -> Result<UserInfo, OidcError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| OidcError::Malformed(e.to_string()))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(std::borrow::ToOwned::to_owned)
    };
    Ok(UserInfo {
        sub: text("sub").ok_or(OidcError::Missing("sub"))?,
        email: text("email"),
        name: text("name"),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SCOPE, OidcError, Pkce, Refresh, RefreshError, access_token_from, authorize_url,
        challenge_for, encode, refresh_if_needed, refresh_now, refresh_request_body,
        token_request_body, token_response_from, user_from,
    };
    use crate::{MemoryTokenStore, StoredSession, TokenStore};

    fn pkce() -> Pkce {
        Pkce::from_entropy([7u8; 32], [9u8; 16])
    }

    /// The RFC 7636 appendix B vector. If this drifts, every exchange
    /// fails with a refusal that names nothing.
    #[test]
    fn the_challenge_matches_the_rfc_vector() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    /// base64url, not base64. `+` and `/` would be re-encoded in the
    /// query string and reach the issuer as something else.
    #[test]
    fn nothing_produces_a_non_url_safe_challenge() {
        for seed in ["a", "ab", "abc", "hello world", "////++++", ""] {
            let challenge = challenge_for(seed);
            assert!(
                !challenge.contains(['+', '/', '=']),
                "{seed:?} produced {challenge}"
            );
        }
    }

    /// 32 bytes base64url'd is 43 characters — the RFC's minimum.
    #[test]
    fn the_verifier_is_long_enough_to_be_legal() {
        let p = pkce();
        assert_eq!(p.verifier().len(), 43);
        assert!(!p.verifier().contains(['+', '/', '=']));
    }

    /// Resuming has to derive the SAME challenge, or the redemption
    /// presents a verifier that does not match what was sent.
    #[test]
    fn resuming_reproduces_the_challenge() {
        let started = pkce();
        let resumed = Pkce::resume(started.verifier(), started.state());
        assert_eq!(started.challenge(), resumed.challenge());
        assert_eq!(started, resumed);
    }

    #[test]
    fn state_is_checked_both_ways() {
        let p = pkce();
        assert!(p.check_state(p.state()).is_ok());
        assert_eq!(
            p.check_state("something else"),
            Err(OidcError::StateMismatch)
        );
        assert_eq!(p.check_state(""), Err(OidcError::StateMismatch));
    }

    #[test]
    fn the_authorize_url_carries_every_required_parameter() {
        let url = authorize_url(
            "https://auth.fasttrackstudio.app",
            "task",
            "https://task.fasttrackstudio.app/auth/callback",
            &pkce(),
            DEFAULT_SCOPE,
        );
        for required in [
            "client_id=task",
            "response_type=code",
            "code_challenge_method=S256",
            "scope=openid%20email%20profile",
        ] {
            assert!(url.contains(required), "{required} missing from {url}");
        }
        // The redirect must be escaped or its :// reads as structure.
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Ftask.fasttrackstudio.app%2Fauth%2Fcallback")
        );
    }

    /// A trailing slash would make the path `//oauth2/authorize`, which
    /// some gateways route differently and none route better.
    #[test]
    fn a_trailing_slash_on_the_issuer_does_not_double_up() {
        let url = authorize_url(
            "https://a.test/",
            "c",
            "https://b.test/cb",
            &pkce(),
            "openid",
        );
        assert!(url.starts_with("https://a.test/oauth2/authorize?"), "{url}");
    }

    #[test]
    fn the_token_body_sends_the_verifier_and_grant_type() {
        let p = pkce();
        let body = token_request_body("task", "https://b.test/cb", "the+code", &p);
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains(&format!("code_verifier={}", p.verifier())));
        // `+` in a code decodes to a space in a form body, so an
        // unescaped one redeems a different code than the app holds.
        assert!(body.contains("code=the%2Bcode"), "got {body}");
    }

    #[test]
    fn unreserved_characters_survive_encoding_untouched() {
        assert_eq!(encode("aZ09-_.~"), "aZ09-_.~");
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("&=?/#"), "%26%3D%3F%2F%23");
    }

    #[test]
    fn the_access_token_is_read_out() {
        assert_eq!(
            access_token_from(r#"{"access_token":"tok","token_type":"Bearer"}"#).unwrap(),
            "tok"
        );
    }

    /// A response that parses but carries nothing usable must fail, not
    /// hand back an empty token that looks like a session.
    #[test]
    fn a_response_without_a_usable_token_is_an_error() {
        for body in [
            r#"{"error":"invalid_grant"}"#,
            r#"{"access_token":""}"#,
            r#"{"access_token":"   "}"#,
            "{}",
        ] {
            assert!(access_token_from(body).is_err(), "{body} should not pass");
        }
        assert!(matches!(
            access_token_from("not json"),
            Err(OidcError::Malformed(_))
        ));
    }

    #[test]
    fn the_whole_token_response_is_read_out() {
        let response = token_response_from(
            r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,
                "token_type":"Bearer","scope":"openid offline_access","id_token":"idt"}"#,
        )
        .unwrap();
        assert_eq!(response.access_token, "at");
        assert_eq!(response.refresh_token.as_deref(), Some("rt"));
        assert_eq!(response.expires_in, Some(3600));
        assert_eq!(response.token_type.as_deref(), Some("Bearer"));
        assert_eq!(response.scope.as_deref(), Some("openid offline_access"));
        assert_eq!(response.id_token.as_deref(), Some("idt"));
        assert_eq!(response.expires_at_unix(1_000), Some(4_600));

        let session = response.into_session(1_000);
        assert_eq!(session.token, "at");
        assert_eq!(session.refresh_token.as_deref(), Some("rt"));
        assert_eq!(session.expires_at_unix, Some(4_600));
    }

    /// Only the access token is required; a blank refresh token is none,
    /// and a string `expires_in` still counts.
    #[test]
    fn a_minimal_token_response_is_enough() {
        let response = token_response_from(r#"{"access_token":"at"}"#).unwrap();
        assert_eq!(response.refresh_token, None);
        assert_eq!(response.expires_in, None);
        assert_eq!(response.expires_at_unix(1_000), None);

        let response =
            token_response_from(r#"{"access_token":"at","refresh_token":" ","expires_in":"60"}"#)
                .unwrap();
        assert_eq!(response.refresh_token, None);
        assert_eq!(response.expires_in, Some(60));
    }

    /// An OAuth error answer says what the issuer said, so a caller can
    /// tell a dead refresh token from a broken response.
    #[test]
    fn an_error_answer_is_a_refusal() {
        assert_eq!(
            token_response_from(
                r#"{"error":"invalid_grant","error_description":"refresh token reused"}"#
            ),
            Err(OidcError::Refused {
                error: "invalid_grant".into(),
                description: Some("refresh token reused".into()),
            })
        );
        assert_eq!(
            token_response_from("{}"),
            Err(OidcError::Missing("access_token"))
        );
    }

    #[test]
    fn the_refresh_body_is_a_public_client_refresh_grant() {
        let body = refresh_request_body("signal", "rt+/=");
        assert_eq!(
            body,
            "grant_type=refresh_token&refresh_token=rt%2B%2F%3D&client_id=signal"
        );
        // A public client: no secret goes out.
        assert!(!body.contains("client_secret"));
    }

    const ROTATED: &str = r#"{"access_token":"at-2","refresh_token":"rt-2","expires_in":3600}"#;

    fn due_session() -> StoredSession {
        StoredSession::new("at-1")
            .with_user_id("u1")
            .with_email("u@x.test")
            .with_expires_at_unix(1_030)
            .with_refresh_token("rt-1")
    }

    #[tokio::test]
    async fn a_due_token_is_rotated_and_saved() {
        let store = MemoryTokenStore::new();
        store.save(&due_session()).unwrap();

        let mut sent = None;
        let outcome = refresh_if_needed(&store, "signal", 1_000, |body| {
            sent = Some(body);
            async { Ok::<_, String>(ROTATED.to_owned()) }
        })
        .await
        .unwrap();

        assert_eq!(
            sent.as_deref(),
            Some("grant_type=refresh_token&refresh_token=rt-1&client_id=signal")
        );
        let saved = store.load().unwrap().unwrap();
        assert_eq!(outcome, Refresh::Refreshed(saved.clone()));
        assert_eq!(saved.token, "at-2");
        assert_eq!(saved.refresh_token.as_deref(), Some("rt-2"));
        assert_eq!(saved.expires_at_unix, Some(4_600));
        assert_eq!(saved.user_id.as_deref(), Some("u1"));
        assert_eq!(saved.email.as_deref(), Some("u@x.test"));
    }

    #[tokio::test]
    async fn nothing_is_sent_when_no_refresh_is_possible_or_needed() {
        let never = |_: String| async { Err::<String, _>("must not send") };

        let store = MemoryTokenStore::new();
        assert_eq!(
            refresh_if_needed(&store, "c", 1_000, never).await.unwrap(),
            Refresh::SignedOut
        );

        let fresh = due_session().with_expires_at_unix(10_000);
        store.save(&fresh).unwrap();
        assert_eq!(
            refresh_if_needed(&store, "c", 1_000, never).await.unwrap(),
            Refresh::Fresh(fresh)
        );

        let no_refresh = StoredSession::new("at").with_expires_at_unix(1_000);
        store.save(&no_refresh).unwrap();
        assert_eq!(
            refresh_if_needed(&store, "c", 1_000, never).await.unwrap(),
            Refresh::NoRefreshToken(no_refresh)
        );
    }

    /// After a 401 the token is renewed whatever its expiry says.
    #[tokio::test]
    async fn refresh_now_ignores_the_expiry() {
        let store = MemoryTokenStore::new();
        store
            .save(&due_session().with_expires_at_unix(10_000))
            .unwrap();
        let outcome = refresh_now(&store, "c", 1_000, |_| async {
            Ok::<_, String>(ROTATED.to_owned())
        })
        .await
        .unwrap();
        assert!(matches!(outcome, Refresh::Refreshed(ref s) if s.token == "at-2"));
        assert_eq!(store.load().unwrap().unwrap().token, "at-2");
    }

    /// A refused or failed refresh leaves the stored session as it was.
    #[tokio::test]
    async fn a_failed_refresh_leaves_the_store_alone() {
        let store = MemoryTokenStore::new();
        store.save(&due_session()).unwrap();

        let refused = refresh_if_needed(&store, "c", 1_000, |_| async {
            Ok::<_, String>(r#"{"error":"invalid_grant"}"#.to_owned())
        })
        .await;
        assert!(matches!(
            refused,
            Err(RefreshError::Oidc(OidcError::Refused { ref error, .. })) if error == "invalid_grant"
        ));

        let failed = refresh_if_needed(&store, "c", 1_000, |_| async {
            Err::<String, _>("offline".to_owned())
        })
        .await;
        assert!(matches!(failed, Err(RefreshError::Send(ref e)) if e == "offline"));

        assert_eq!(store.load().unwrap(), Some(due_session()));
    }

    /// A person with no display name still signs in.
    #[test]
    fn userinfo_needs_only_a_subject() {
        let user = user_from(r#"{"sub":"abc-123"}"#).unwrap();
        assert_eq!(user.sub, "abc-123");
        assert_eq!(user.email, None);
        assert_eq!(user.name, None);

        let full = user_from(r#"{"sub":"a","email":"e@x.test","name":"Nom"}"#).unwrap();
        assert_eq!(full.email.as_deref(), Some("e@x.test"));
        assert_eq!(full.name.as_deref(), Some("Nom"));
    }

    /// Blank is not a value: a name of `" "` would render as an empty
    /// account label rather than falling back to the address.
    #[test]
    fn blank_claims_are_treated_as_absent() {
        let user = user_from(r#"{"sub":"a","email":"","name":"   "}"#).unwrap();
        assert_eq!(user.email, None);
        assert_eq!(user.name, None);
    }

    #[test]
    fn userinfo_without_a_subject_is_an_error() {
        assert_eq!(
            user_from(r#"{"email":"e@x.test"}"#),
            Err(OidcError::Missing("sub"))
        );
    }
}
