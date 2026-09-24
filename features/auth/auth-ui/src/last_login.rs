//! "You signed in with GitHub last time."
//!
//! # Why a cookie
//!
//! The engine records the method against the *user*, which is exactly
//! the wrong side of the problem: the person reading a sign-in page has
//! no session, so there is nobody to look it up for. The answer has to
//! live in the browser. So every path that signs somebody in leaves a
//! small non-secret cookie behind, and the sign-in page reads it.
//!
//! # Why it is worth the trouble
//!
//! A sign-in screen with five buttons is a memory test, and the usual
//! way of failing it is expensive: somebody who signed up with Google
//! tries the password form, fails, resets a password they never had,
//! and ends up with a second account. A one-line hint turns that into
//! a glance.
//!
//! # What it may not be
//!
//! It names a *method*, never an address, and it is set for everyone
//! who signs in successfully — so it says nothing about whether any
//! particular account exists. It is not `HttpOnly`, because a
//! first-party script may reasonably want to highlight the same button;
//! nothing about it is a credential, and treating it as one would be
//! cargo cult.

use architect_auth::flows::last_login_method::{
    default_cookie_config, last_login_method_from_user,
};
use architect_auth::proto::AuthUser;
use axum::http::HeaderMap;

/// The `Set-Cookie` value remembering how this person just signed in.
///
/// `None` when the engine recorded nothing — an impersonated session,
/// say, which must not overwrite the operator's own hint.
#[must_use]
pub fn remember(user: &AuthUser) -> Option<String> {
    let method = last_login_method_from_user(user).ok().flatten()?;
    let config = default_cookie_config();
    Some(format!(
        "{}={}; Path=/; Max-Age={}; SameSite=Lax",
        config.name,
        // The recorded methods are short lowercase identifiers, but a
        // provider id is configuration, so it is filtered rather than
        // trusted: anything else would let a provider name inject
        // cookie attributes.
        method
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>(),
        config.max_age_seconds,
    ))
}

/// How this browser signed in last, if it has before.
#[must_use]
pub fn recall(headers: &HeaderMap) -> Option<String> {
    let name = default_cookie_config().name;
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
        .filter(|value| !value.is_empty())
}

/// The method, in words, for the sentence "You last signed in **…**".
///
/// Unknown values fall through to the raw identifier rather than being
/// dropped: a provider this build has never heard of is still better
/// named than not named.
#[must_use]
pub fn describe(method: &str) -> String {
    match method {
        "email" => "with an email address and password".to_owned(),
        "username" => "with a username and password".to_owned(),
        "passkey" => "with a passkey".to_owned(),
        "magic-link" => "with an emailed link".to_owned(),
        "email-otp" => "with an emailed code".to_owned(),
        "phone-number" => "with a phone number".to_owned(),
        "siwe" => "with a wallet".to_owned(),
        "anonymous" => "as a guest".to_owned(),
        "github" => "with GitHub".to_owned(),
        "google" => "with Google".to_owned(),
        "tone3000" => "with TONE3000".to_owned(),
        other => format!("with {other}"),
    }
}

#[cfg(test)]
mod tests {
    use architect_auth::proto::AuthUser;
    use axum::http::{HeaderMap, HeaderValue, header};

    use super::{describe, recall, remember};

    fn user(metadata: &str) -> AuthUser {
        AuthUser {
            id: uuid::Uuid::new_v4(),
            email: Some("ada@example.com".into()),
            name: None,
            email_verified: true,
            image: None,
            username: None,
            display_username: None,
            two_factor_enabled: false,
            role: None,
            banned: false,
            ban_reason: None,
            ban_expires: None,
            metadata_json: metadata.to_owned(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_recorded_method_becomes_a_cookie() {
        let cookie = remember(&user(r#"{"last_login_method":"github"}"#)).expect("a cookie");
        assert!(cookie.contains("=github;"), "{cookie}");
        assert!(cookie.contains("Path=/"), "{cookie}");
        assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    }

    #[test]
    fn nothing_recorded_means_no_cookie() {
        // An impersonated session records nothing, and must not
        // overwrite the operator's own hint.
        assert!(remember(&user("{}")).is_none());
        assert!(remember(&user("not json")).is_none());
    }

    #[test]
    fn a_hostile_provider_name_cannot_add_cookie_attributes() {
        // Provider ids are configuration, so they are filtered rather
        // than trusted.
        let cookie = remember(&user(
            r#"{"last_login_method":"evil; Domain=attacker.example"}"#,
        ))
        .expect("a cookie");
        assert!(!cookie.contains("attacker.example"), "{cookie}");
        assert!(cookie.contains("=evilDomainattackerexample;"), "{cookie}");
    }

    #[test]
    fn the_cookie_reads_back_out_of_a_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; better-auth.last_used_login_method=passkey; x=2"),
        );
        assert_eq!(recall(&headers).as_deref(), Some("passkey"));
    }

    #[test]
    fn no_cookie_is_no_hint() {
        assert_eq!(recall(&HeaderMap::new()), None);
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static("unrelated=1"));
        assert_eq!(recall(&headers), None);
    }

    #[test]
    fn every_method_the_engine_records_has_words_for_it() {
        // Exactly the list `record_last_login_method` is called with,
        // plus the provider ids this deployment configures. A method
        // with no wording falls through to `with <identifier>`, which
        // reads as a leak of an internal name on a sign-in page.
        for method in [
            "email",
            "username",
            "passkey",
            "magic-link",
            "email-otp",
            "phone-number",
            "siwe",
            "anonymous",
            "github",
            "google",
            "tone3000",
        ] {
            assert_ne!(
                describe(method),
                format!("with {method}"),
                "{method} has no wording of its own"
            );
        }
        // And something unheard-of is still named rather than dropped.
        assert_eq!(describe("okta"), "with okta");
    }
}
