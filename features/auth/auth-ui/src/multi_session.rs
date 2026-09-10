//! Being signed in as more than one person at once.
//!
//! # Where the list lives
//!
//! `list_device_sessions` takes a *list of tokens* — the engine holds
//! no notion of "this browser's other accounts", it just resolves
//! whatever it is handed and drops what has expired. So the list has to
//! live in the browser, which means a second cookie alongside the
//! session one.
//!
//! That shape is right for the same reason the engine chose it: the
//! server has no business knowing that two accounts share a laptop.
//! A work account and a personal one are linked by nothing except the
//! browser they are both open in, and that link disappears when the
//! browser does.
//!
//! # Bounded on purpose
//!
//! At most [`MAX_ACCOUNTS`] tokens, oldest dropped first. Each is
//! roughly forty characters and cookies travel on every request to this
//! origin, so an unbounded roster is a slow leak into the size of every
//! request. Somebody with more accounts than that signs in again, which
//! is the same thing they do today.

use architect_auth::transport::AuthCookieConfig;
use axum::http::{HeaderMap, header};
use axum::response::Response;

/// How many accounts one browser may hold at once.
pub const MAX_ACCOUNTS: usize = 5;

/// The roster cookie's name, derived from the session cookie's.
///
/// Derived rather than configured: the two are meaningless apart, and a
/// deployment that renamed one and forgot the other would have a
/// browser quietly holding a roster it could never read.
#[must_use]
pub fn roster_cookie_name(session_cookie: &str) -> String {
    format!("{session_cookie}.accounts")
}

/// Every session token this browser is holding, newest first.
///
/// Includes the current one when it is in the roster, which it will be
/// — the point is that switching is a matter of promoting one of these,
/// not of signing in again.
#[must_use]
pub fn roster(headers: &HeaderMap, session_cookie: &str) -> Vec<String> {
    let name = roster_cookie_name(session_cookie);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| {
            value
                .split(',')
                .map(str::trim)
                .filter(|token| !token.is_empty() && is_token_shaped(token))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Add a token to the roster, keeping it first and bounded.
pub fn remember(
    response: &mut Response,
    cookie: &AuthCookieConfig,
    headers: &HeaderMap,
    token: &str,
) {
    let mut tokens = roster(headers, &cookie.name);
    tokens.retain(|existing| existing != token);
    tokens.insert(0, token.to_owned());
    tokens.truncate(MAX_ACCOUNTS);
    set_roster(response, cookie, &tokens);
}

/// Drop a token from the roster — a sign-out, or a revoked session.
pub fn forget(
    response: &mut Response,
    cookie: &AuthCookieConfig,
    headers: &HeaderMap,
    token: &str,
) {
    let mut tokens = roster(headers, &cookie.name);
    tokens.retain(|existing| existing != token);
    set_roster(response, cookie, &tokens);
}

/// Forget every held account.
pub fn clear(response: &mut Response, cookie: &AuthCookieConfig) {
    set_roster(response, cookie, &[]);
}

fn set_roster(response: &mut Response, cookie: &AuthCookieConfig, tokens: &[String]) {
    let value = tokens.join(",");
    // `Max-Age=0` on an empty roster: an empty cookie that lingers is
    // just a request header nobody reads.
    let age = if tokens.is_empty() {
        "Max-Age=0".to_owned()
    } else {
        cookie
            .max_age_seconds
            .map_or_else(|| "Max-Age=2592000".to_owned(), |s| format!("Max-Age={s}"))
    };
    let built = format!(
        "{}={value}; Path={}; {age}; SameSite=Lax; HttpOnly{}",
        roster_cookie_name(&cookie.name),
        cookie.path,
        if cookie.secure { "; Secure" } else { "" },
    );
    if let Ok(header_value) = built.parse() {
        response
            .headers_mut()
            .append(header::SET_COOKIE, header_value);
    }
}

/// Does this look like a session token and nothing else?
///
/// The roster is read back out of a header and split on commas, so a
/// value that carried a `;` or a space would be a way to append cookie
/// attributes. Tokens are base64url, so anything outside that alphabet
/// is not one.
fn is_token_shaped(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use architect_auth::transport::AuthCookieConfig;
    use axum::http::{HeaderMap, HeaderValue, header};
    use axum::response::{IntoResponse as _, Response};

    use super::{MAX_ACCOUNTS, forget, remember, roster, roster_cookie_name};

    fn config() -> AuthCookieConfig {
        AuthCookieConfig {
            name: "sess".into(),
            ..AuthCookieConfig::default()
        }
    }

    fn with_roster(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("sess=current; sess.accounts={value}")).unwrap(),
        );
        headers
    }

    fn roster_of(response: &Response) -> String {
        let name = roster_cookie_name("sess");
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&name))
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn a_new_token_goes_to_the_front_and_is_not_duplicated() {
        let mut response = ().into_response();
        remember(&mut response, &config(), &with_roster("bbb,aaa"), "aaa");
        assert!(
            roster_of(&response).starts_with("sess.accounts=aaa,bbb;"),
            "{}",
            roster_of(&response)
        );
    }

    #[test]
    fn the_roster_is_bounded() {
        let existing = (0..MAX_ACCOUNTS)
            .map(|i| format!("token{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut response = ().into_response();
        remember(&mut response, &config(), &with_roster(&existing), "newest");
        let cookie = roster_of(&response);
        let value = cookie
            .split_once('=')
            .and_then(|(_, rest)| rest.split(';').next())
            .unwrap_or_default();
        assert_eq!(value.split(',').count(), MAX_ACCOUNTS, "{cookie}");
        assert!(value.starts_with("newest,"), "{cookie}");
        // The oldest fell off the end.
        assert!(
            !value.contains(&format!("token{}", MAX_ACCOUNTS - 1)),
            "{cookie}"
        );
    }

    #[test]
    fn signing_out_removes_only_that_one() {
        let mut response = ().into_response();
        forget(&mut response, &config(), &with_roster("aaa,bbb,ccc"), "bbb");
        assert!(
            roster_of(&response).starts_with("sess.accounts=aaa,ccc;"),
            "{}",
            roster_of(&response)
        );
    }

    #[test]
    fn an_emptied_roster_is_asked_to_go_away() {
        let mut response = ().into_response();
        forget(&mut response, &config(), &with_roster("aaa"), "aaa");
        assert!(
            roster_of(&response).contains("Max-Age=0"),
            "{}",
            roster_of(&response)
        );
    }

    #[test]
    fn a_value_that_is_not_token_shaped_is_dropped_on_the_way_in() {
        // Two layers, and it matters which does what. A `;` never
        // reaches this filter at all — cookie parsing splits pairs on
        // it first, so everything after one is a different cookie and
        // not the roster. What the filter catches is the rest: spaces,
        // quotes, anything outside the base64url alphabet a token uses.
        let tokens = roster(&with_roster("good,bad value,also\"bad,fine"), "sess");
        assert_eq!(tokens, vec!["good".to_owned(), "fine".to_owned()]);

        // And the `;` case, which the header split handles: whatever
        // follows is simply not part of this cookie's value.
        let tokens = roster(&with_roster("good,worse; Domain=evil.example"), "sess");
        assert_eq!(tokens, vec!["good".to_owned(), "worse".to_owned()]);
    }

    #[test]
    fn no_cookie_is_an_empty_roster() {
        assert!(roster(&HeaderMap::new(), "sess").is_empty());
    }
}
