//! The app's sign-in by redirect (authorization code + PKCE, S256).
//!
//! The parts with no network: where the issuer sends the browser back, the
//! PKCE pair, reading the callback (its `state` checked before anything
//! is redeemed), and the token endpoint's forms.
//!
//! The flow (`Accounts::sign_in_browser`):
//!
//! 1. The issuer's `/oauth2/authorize` opens in the system's browser
//!    (`crate::browser`): iOS's sign-in sheet, a desk's default browser.
//! 2. The person signs in there and the issuer sends the browser to the
//!    app's redirect: [`SCHEME_REDIRECT`] on iOS (the sheet hands it to the
//!    app), a loopback port of the app's own on a desk
//!    ([`loopback_redirect`]; the issuer takes any port, RFC 8252 §7.3).
//! 3. The app checks the `state` ([`callback`]), redeems the code with its
//!    verifier ([`auth_client::oidc::token_request_body`]) and trades the
//!    fresh access token for an issuer session ([`exchange_body`], RFC
//!    8693; the issuer allows it to the `architect` client alone). The
//!    session is kept as every session is (the keychain).

use std::fmt::Write as _;

use auth_client::oidc::Pkce;

/// Where iOS's sheet returns: the app's own scheme (reverse DNS, RFC 8252
/// §7.1), registered at the issuer for `architect`.
pub const SCHEME_REDIRECT: &str = "app.fasttrackstudio.architect:/oauth/callback";

/// The path of the desk's loopback redirect.
pub const LOOPBACK_PATH: &str = "/oauth/callback";

/// What is asked for: who the person is, and a refresh token.
pub const SCOPE: &str = "openid email profile offline_access";

/// The loopback redirect on `port`, as registered at the issuer
/// (`http://127.0.0.1/oauth/callback`, any port).
#[must_use]
pub fn loopback_redirect(port: u16) -> String {
    format!("http://127.0.0.1:{port}{LOOPBACK_PATH}")
}

/// A fresh PKCE verifier and state, from the system's randomness (uuid
/// v4 draws on the OS's generator).
#[must_use]
pub fn new_pkce() -> Pkce {
    let (a, b, c) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let mut verifier = [0_u8; 32];
    verifier[..16].copy_from_slice(a.as_bytes());
    verifier[16..].copy_from_slice(b.as_bytes());
    Pkce::from_entropy(verifier, *c.as_bytes())
}

/// Why a callback was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackError {
    /// The issuer said no (`error=access_denied`, …).
    #[error("the issuer refused: {0}")]
    Refused(String),
    /// The `state` is not this attempt's: a stale or forged callback.
    #[error("this sign-in was not the one the app started")]
    WrongState,
    #[error("the issuer sent no code back")]
    NoCode,
}

/// The code in `url` (the redirect the browser landed on, or its path and
/// query), once its `state` is `pkce`'s.
///
/// # Errors
///
/// [`CallbackError`]: the issuer's refusal, another attempt's state, no
/// code.
pub fn callback(url: &str, pkce: &Pkce) -> Result<String, CallbackError> {
    let query = url.split_once('?').map_or("", |(_, q)| q);
    let query = query.split_once('#').map_or(query, |(q, _)| q);
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = decode(v);
        match k {
            "code" => code = Some(v),
            "state" => state = Some(v),
            "error" => error = Some(v),
            _ => {}
        }
    }
    if let Some(e) = error {
        return Err(CallbackError::Refused(e));
    }
    if pkce.check_state(state.as_deref().unwrap_or_default()).is_err() {
        return Err(CallbackError::WrongState);
    }
    code.filter(|c| !c.is_empty()).ok_or(CallbackError::NoCode)
}

/// The token exchange's form (RFC 8693): `access_token` traded for an
/// issuer session.
#[must_use]
pub fn exchange_body(client_id: &str, access_token: &str) -> String {
    form(&[
        ("grant_type", "urn:ietf:params:oauth:grant-type:token-exchange"),
        ("client_id", client_id),
        ("subject_token", access_token),
        ("subject_token_type", "urn:ietf:params:oauth:token-type:access_token"),
        (
            "requested_token_type",
            "urn:fasttrackstudio:params:oauth:token-type:session",
        ),
    ])
}

/// `application/x-www-form-urlencoded`.
fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes
                    .get(i.saturating_add(1)..i.saturating_add(3))
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(v) = hex {
                    out.push(v);
                    i = i.saturating_add(2);
                } else {
                    out.push(b'%');
                }
            }
            _ => out.push(b),
        }
        i = i.saturating_add(1);
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the desk's loopback listener answers the browser with.
pub const LOOPBACK_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Architect</title>\
<body style=\"font:16px -apple-system,system-ui,sans-serif;padding:48px;color:#e4e4e7;background:#09090b\">\
<h1 style=\"font-size:22px\">You're signed in</h1><p>You can close this tab and go back to Architect.</p>";

/// The request target of an HTTP request's first line
/// (`GET /oauth/callback?code=… HTTP/1.1`), when it is a GET of the
/// loopback redirect's path.
#[must_use]
pub fn loopback_target(request: &str) -> Option<&str> {
    let line = request.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    let path = target.split_once('?').map_or(target, |(p, _)| p);
    (path == LOOPBACK_PATH).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkce() -> Pkce {
        Pkce::resume("verifier", "st8")
    }

    #[test]
    fn a_callback_gives_its_code_only_for_this_attempts_state() {
        let p = pkce();
        assert_eq!(
            callback("app.fasttrackstudio.architect:/oauth/callback?code=c0de&state=st8", &p),
            Ok("c0de".to_owned())
        );
        assert_eq!(callback("/oauth/callback?state=st8&code=a%2Bb", &p), Ok("a+b".to_owned()));
        // Another attempt's state (a stale tab, a forged link).
        assert_eq!(callback("/oauth/callback?code=c0de&state=other", &p), Err(CallbackError::WrongState));
        assert_eq!(callback("/oauth/callback?code=c0de", &p), Err(CallbackError::WrongState));
        // The issuer said no.
        assert_eq!(
            callback("/oauth/callback?error=access_denied&state=st8", &p),
            Err(CallbackError::Refused("access_denied".to_owned()))
        );
        assert_eq!(callback("/oauth/callback?state=st8", &p), Err(CallbackError::NoCode));
    }

    #[test]
    fn the_loopback_listener_takes_only_its_callback() {
        assert_eq!(
            loopback_target("GET /oauth/callback?code=x&state=y HTTP/1.1\r\nHost: 127.0.0.1\r\n"),
            Some("/oauth/callback?code=x&state=y")
        );
        assert_eq!(loopback_target("GET /favicon.ico HTTP/1.1\r\n"), None);
        assert_eq!(loopback_target("POST /oauth/callback HTTP/1.1\r\n"), None);
        assert_eq!(loopback_redirect(53124), "http://127.0.0.1:53124/oauth/callback");
    }

    #[test]
    fn the_exchange_form_names_the_session_it_asks_for() {
        let body = exchange_body("architect", "eyJ.a-b_c");
        assert!(body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange"));
        assert!(body.contains("subject_token=eyJ.a-b_c"));
        assert!(body.contains("requested_token_type=urn%3Afasttrackstudio%3Aparams%3Aoauth%3Atoken-type%3Asession"));
        // Two attempts never share a verifier or a state.
        let (a, b) = (new_pkce(), new_pkce());
        assert_ne!(a.verifier(), b.verifier());
        assert_ne!(a.state(), b.state());
        assert!(a.verifier().len() >= 43, "RFC 7636's shortest verifier");
    }
}
