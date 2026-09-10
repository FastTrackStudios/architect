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
        body {
            Shell { {body} }
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

/// The session token on this request, from `Authorization` or the cookie.
#[must_use]
pub fn token_of(headers: &HeaderMap, cookie: &AuthCookieConfig) -> Option<String> {
    session_token_from_headers(headers, cookie)
}

#[cfg(test)]
mod tests {
    use super::Flash;

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
