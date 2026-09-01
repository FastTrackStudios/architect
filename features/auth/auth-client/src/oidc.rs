//! The redirect sign-in, minus the plumbing.
//!
//! Every FastTrackStudio app — Task, Session, Signal, Keyflow, Ignition
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
//!   things, then read the answers with [`access_token_from`] and
//!   [`user_from`].
//! * **somewhere to park the verifier** while the browser is away.
//!   `sessionStorage` is the right answer on web: the verifier is
//!   meaningful for one attempt in one tab, and `localStorage` would
//!   leave it readable by every later page load in the origin.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

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

/// Read the access token out of a `/oauth2/token` response.
///
/// # Errors
/// [`OidcError::Malformed`] if it is not JSON, [`OidcError::Missing`] if
/// there is no usable `access_token` — including one present but empty,
/// which would otherwise pass for a signed-in session.
pub fn access_token_from(body: &str) -> Result<String, OidcError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| OidcError::Malformed(e.to_string()))?;
    value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map(std::borrow::ToOwned::to_owned)
        .ok_or(OidcError::Missing("access_token"))
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
        DEFAULT_SCOPE, OidcError, Pkce, access_token_from, authorize_url, challenge_for, encode,
        token_request_body, user_from,
    };

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
        assert_eq!(p.check_state("something else"), Err(OidcError::StateMismatch));
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
        let url = authorize_url("https://a.test/", "c", "https://b.test/cb", &pkce(), "openid");
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
