//! Turning a component into a response, and the bits every page needs.

use architect_auth::transport::AuthCookieConfig;
use architect_auth::transport::axum::session_token_from_headers;
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Redirect, Response};
use dioxus::prelude::*;

use crate::chrome::{STYLE, Shell};

/// A one-line message at the top of a page, from the query string the
/// previous POST redirected with.
///
/// Redirect-with-a-flash rather than rendering the outcome directly:
/// a page rendered straight from a POST re-submits on refresh, and the
/// second submission of "remove this member" is nobody's intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flash {
    Ok(String),
    Error(String),
}

impl Flash {
    /// Read `?ok=` / `?error=` into a flash, preferring the error.
    #[must_use]
    pub fn from_query(ok: Option<&str>, error: Option<&str>) -> Option<Self> {
        error
            .filter(|s| !s.is_empty())
            .map(|s| Self::Error(s.to_owned()))
            .or_else(|| ok.filter(|s| !s.is_empty()).map(|s| Self::Ok(s.to_owned())))
    }
}

/// Render a page body into a full HTML document.
pub fn document(title: &str, body: Element) -> Response {
    let rendered = dioxus_ssr::render_element(rsx! {
        head {
            meta { charset: "utf-8" }
            meta { name: "viewport", content: "width=device-width, initial-scale=1" }
            title { "{title} · FastTrackStudio" }
            style { {STYLE} }
        }
        body { class: "auth-card",
            Shell { {body} }
        }
    });
    Html(format!(
        "<!doctype html>\n<html lang=\"en\">{rendered}</html>"
    ))
    .into_response()
}

/// As [`document`], with a script in the page.
///
/// Only the passkey pages use this: `navigator.credentials` cannot be
/// reached from a form, so a passkey needs script no matter what. It
/// is inlined rather than fetched for the same reason the stylesheet
/// is — one document, no second request to fail.
pub fn document_with_script(title: &str, body: Element, script: &str) -> Response {
    let rendered = dioxus_ssr::render_element(rsx! {
        head {
            meta { charset: "utf-8" }
            meta { name: "viewport", content: "width=device-width, initial-scale=1" }
            title { "{title} · FastTrackStudio" }
            style { {STYLE} }
        }
        body { class: "auth-card",
            Shell { {body} }
            script { dangerous_inner_html: "{script}" }
        }
    });
    Html(format!(
        "<!doctype html>\n<html lang=\"en\">{rendered}</html>"
    ))
    .into_response()
}

/// Redirect back to `path`, carrying a flash message.
#[must_use]
pub fn flash_to(path: &str, flash: &Flash) -> Response {
    let (key, message) = match flash {
        Flash::Ok(message) => ("ok", message),
        Flash::Error(message) => ("error", message),
    };
    let sep = if path.contains('?') { '&' } else { '?' };
    let encoded = architect_auth::percent::encode_component(message);
    Redirect::to(&format!("{path}{sep}{key}={encoded}")).into_response()
}

/// Send a browser with no session to sign in, and back here afterwards.
#[must_use]
pub fn sign_in_first(return_to: &str) -> Response {
    let encoded = architect_auth::percent::encode_component(return_to);
    Redirect::to(&format!("/login?return_to={encoded}")).into_response()
}

/// Reduce a caller-supplied path to something safe to `Location:`.
///
/// An unchecked `return_to` is an open redirect, and on an *identity*
/// server that is worth more than usual: the phishing page it forwards
/// to is reached through a link that genuinely begins at the real login
/// screen, having genuinely signed the person in.
///
/// Only same-origin absolute paths survive. `//evil.example` is
/// rejected along with every scheme-bearing URL — a leading `//` is a
/// protocol-relative URL, which browsers treat as cross-origin even
/// though it looks like a path. Backslashes and newlines go too: the
/// first because some browsers normalise `\` to `/`, the second
/// because a newline in a header value splits the response.
#[must_use]
pub fn safe_path(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or("").trim();
    let ok = candidate.starts_with('/')
        && !candidate.starts_with("//")
        && !candidate.contains(['\\', '\r', '\n']);
    if ok {
        candidate.to_owned()
    } else {
        "/".to_owned()
    }
}

/// The session token on this request, from `Authorization` or the cookie.
#[must_use]
pub fn token_of(headers: &HeaderMap, cookie: &AuthCookieConfig) -> Option<String> {
    session_token_from_headers(headers, cookie)
}

#[cfg(test)]
mod tests {
    use super::{Flash, safe_path};

    #[test]
    fn only_a_same_origin_path_survives() {
        assert_eq!(safe_path(Some("/orgs")), "/orgs");
        assert_eq!(safe_path(Some("/a?b=c&d=e")), "/a?b=c&d=e");
    }

    #[test]
    fn an_open_redirect_is_refused() {
        // Each of these would forward somebody off an identity server
        // through a link that genuinely began at the real login screen.
        for hostile in [
            "//evil.example",
            "https://evil.example",
            "javascript:alert(1)",
            "/\\evil.example",
            "/ok\r\nSet-Cookie: a=b",
            "",
            "   ",
        ] {
            assert_eq!(
                safe_path(Some(hostile)),
                "/",
                "{hostile:?} must not survive"
            );
        }
        assert_eq!(safe_path(None), "/");
    }

    #[test]
    fn an_error_wins_over_an_ok() {
        // Both arrive when a partial success redirects with a warning;
        // showing the cheerful one would bury it.
        assert_eq!(
            Flash::from_query(Some("saved"), Some("nope")),
            Some(Flash::Error("nope".into()))
        );
    }

    #[test]
    fn an_empty_parameter_is_not_a_message() {
        assert_eq!(Flash::from_query(Some(""), Some("")), None);
        assert_eq!(Flash::from_query(None, None), None);
    }
}
