//! The pages a person actually sees.
//!
//! Everything else in this crate is an API: JSON in, JSON out, for a
//! program that already holds a credential. But `/oauth2/authorize`
//! needs a *browser* to be signed in before it can issue a code, and
//! until this module existed there was nowhere for that to happen — the
//! handler simply answered `401` and the redirect flow dead-ended. Its
//! own doc comment said "a front-end owns that"; this is that front-end.
//!
//! # Server-rendered, and no script at all
//!
//! Rendered with `dioxus-ssr` into plain HTML, with ordinary `<form>`
//! posts. That is a deliberate choice for *these* pages: a sign-in
//! screen is the one page that must work before anything else does, and
//! making someone download and boot a WASM bundle before they can type a
//! password buys nothing and fails badly — a slow network, a blocked
//! script, or a panic in the bundle all become "I cannot log in". A form
//! post has none of those failure modes.
//!
//! # Why the form posts here and not to the JSON endpoints
//!
//! `/auth/sign-in/email` takes JSON and answers JSON, which a browser
//! form cannot send and a person cannot read. These routes take
//! `application/x-www-form-urlencoded`, call exactly the same engine
//! methods, and answer with a redirect — so the two surfaces cannot
//! drift, because there is only one implementation underneath.

use architect_auth::{AuthStorage, CreateEmailPasswordUser, transport::AuthCookieConfig};
use auth_proto::{AuthSessionBundle, SignInEmailPassword};
use axum::{
    Router,
    extract::{Form, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use dioxus::prelude::*;

use crate::http::{HttpState, client_ip, user_agent};

/// Where to send a browser that has just signed in, when the request
/// carried no `return_to` of its own.
const DEFAULT_AFTER_SIGN_IN: &str = "/";

/// Mount the sign-in and sign-up pages.
///
/// Merged next to [`crate::http::router`] rather than folded into it:
/// an embedder that already has its own login screen wants the API
/// without these, and keeping them separable is the difference between
/// "use our pages" and "use our server".
pub fn router<S>(state: HttpState<S>) -> Router
where
    S: AuthStorage + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/login", get(login_page::<S>).post(login_submit::<S>))
        .route("/sign-up", get(sign_up_page::<S>).post(sign_up_submit::<S>))
        .with_state(state)
}

// ── Redirect safety ──────────────────────────────────────────────────

/// Reduce a caller-supplied `return_to` to something safe to `Location:`.
///
/// An unchecked `return_to` is an open redirect, and on an *identity*
/// server that is worth more than usual: the phishing page it forwards
/// to is reached through a link that genuinely begins at the real login
/// screen, having genuinely signed the person in.
///
/// Only same-origin absolute paths survive. `//evil.example` is rejected
/// along with every scheme-bearing URL — a leading `//` is a
/// protocol-relative URL, which browsers treat as cross-origin even
/// though it looks like a path.
fn safe_return_to(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or("").trim();
    let ok = candidate.starts_with('/')
        && !candidate.starts_with("//")
        && !candidate.contains(['\\', '\r', '\n']);
    if ok {
        candidate.to_owned()
    } else {
        DEFAULT_AFTER_SIGN_IN.to_owned()
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub return_to: Option<String>,
}

// ── GET ──────────────────────────────────────────────────────────────

async fn login_page<S>(Query(q): Query<PageQuery>) -> Html<String>
where
    S: AuthStorage,
{
    Html(render(Screen::SignIn, &safe_return_to(q.return_to.as_deref()), None))
}

async fn sign_up_page<S>(Query(q): Query<PageQuery>) -> Html<String>
where
    S: AuthStorage,
{
    Html(render(Screen::SignUp, &safe_return_to(q.return_to.as_deref()), None))
}

// ── POST ─────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct SignInForm {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct SignUpForm {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub return_to: Option<String>,
}

async fn login_submit<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Form(form): Form<SignInForm>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_return_to(form.return_to.as_deref());
    let result = state
        .auth
        .sign_in_email_password(SignInEmailPassword {
            email: form.email,
            password: form.password,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await;

    match result {
        Ok(bundle) => signed_in(&state.cookie, &bundle, &return_to),
        // Deliberately not distinguishing "no such account" from "wrong
        // password": the difference is an account-enumeration oracle,
        // and a person who mistyped either one does the same thing next.
        Err(_) => rejected(Screen::SignIn, &return_to, "Email or password is incorrect."),
    }
}

async fn sign_up_submit<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Form(form): Form<SignUpForm>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_return_to(form.return_to.as_deref());

    // Checked here rather than only in the browser, because `required`
    // and `minlength` on the input are a convenience for the person, not
    // a control — this endpoint is reachable without them.
    if form.password.chars().count() < 8 {
        return rejected(
            Screen::SignUp,
            &return_to,
            "Password must be at least 8 characters.",
        );
    }

    let result = state
        .auth
        .create_email_password_user(CreateEmailPasswordUser {
            email: form.email,
            password: form.password,
            name: form.name.filter(|n| !n.trim().is_empty()),
            username: None,
            image: None,
            metadata_json: None,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await;

    match result {
        Ok(bundle) => signed_in(&state.cookie, &bundle, &return_to),
        Err(e) => rejected(Screen::SignUp, &return_to, &describe_sign_up_error(&e)),
    }
}

/// A sign-up failure a person can act on.
///
/// Unlike sign-in, saying "that address is taken" is the *useful* answer
/// and leaks nothing a sign-up attempt would not reveal anyway.
fn describe_sign_up_error(error: &impl std::fmt::Display) -> String {
    let text = error.to_string();
    let lower = text.to_lowercase();
    if lower.contains("exist") || lower.contains("taken") || lower.contains("duplicate") {
        "An account with that email already exists — sign in instead.".to_owned()
    } else {
        text
    }
}

/// Set the session cookie and send the browser on.
///
/// 303, not 302: the browser must switch to GET for the redirect, or a
/// refresh on the destination re-submits the credentials.
fn signed_in(cookie: &AuthCookieConfig, bundle: &AuthSessionBundle, return_to: &str) -> Response {
    let set_cookie = cookie.session_cookie(bundle.token.clone());
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, set_cookie.to_string()),
            (header::LOCATION, return_to.to_owned()),
        ],
    )
        .into_response()
}

/// Re-render the screen with the reason it failed.
///
/// 200, not 4xx: this is a page a person reads, and some browsers and
/// intermediaries substitute their own body for an error status.
fn rejected(screen: Screen, return_to: &str, message: &str) -> Response {
    Html(render(screen, return_to, Some(message))).into_response()
}

// ── Rendering ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    SignIn,
    SignUp,
}

impl Screen {
    fn title(self) -> &'static str {
        match self {
            Screen::SignIn => "Sign in",
            Screen::SignUp => "Create your account",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Screen::SignIn => "/login",
            Screen::SignUp => "/sign-up",
        }
    }

    fn submit(self) -> &'static str {
        match self {
            Screen::SignIn => "Sign in",
            Screen::SignUp => "Create account",
        }
    }
}

fn render(screen: Screen, return_to: &str, error: Option<&str>) -> String {
    let body = dioxus_ssr::render_element(rsx! {
        Page {
            screen,
            return_to: return_to.to_owned(),
            error: error.map(std::borrow::ToOwned::to_owned),
        }
    });
    format!("<!doctype html>\n<html lang=\"en\">{body}</html>")
}

#[component]
fn Page(screen: Screen, return_to: String, error: Option<String>) -> Element {
    rsx! {
        head {
            meta { charset: "utf-8" }
            meta {
                name: "viewport",
                content: "width=device-width, initial-scale=1",
            }
            title { "{screen.title()} · FastTrackStudio" }
            style { {STYLE} }
        }
        body {
            main { class: "card",
                h1 { "{screen.title()}" }
                p { class: "sub", "One account for every FastTrackStudio app." }

                if let Some(message) = error {
                    p { class: "error", role: "alert", "{message}" }
                }

                form { method: "post", action: screen.action(),
                    input { r#type: "hidden", name: "return_to", value: "{return_to}" }

                    if screen == Screen::SignUp {
                        label { r#for: "name", "Name" }
                        input {
                            id: "name",
                            name: "name",
                            r#type: "text",
                            autocomplete: "name",
                        }
                    }

                    label { r#for: "email", "Email" }
                    input {
                        id: "email",
                        name: "email",
                        r#type: "email",
                        autocomplete: "email",
                        required: true,
                        autofocus: true,
                    }

                    label { r#for: "password", "Password" }
                    input {
                        id: "password",
                        name: "password",
                        r#type: "password",
                        // Tells a password manager to offer a new
                        // password on sign-up and the saved one on
                        // sign-in; the wrong value here is why managers
                        // sometimes refuse to fill.
                        autocomplete: if screen == Screen::SignUp { "new-password" } else { "current-password" },
                        required: true,
                        minlength: if screen == Screen::SignUp { "8" } else { "1" },
                    }

                    button { r#type: "submit", "{screen.submit()}" }
                }

                p { class: "alt",
                    match screen {
                        Screen::SignIn => rsx! {
                            "No account yet? "
                            a { href: "/sign-up?return_to={return_to}", "Create one" }
                        },
                        Screen::SignUp => rsx! {
                            "Already have an account? "
                            a { href: "/login?return_to={return_to}", "Sign in" }
                        },
                    }
                }
            }
        }
    }
}

/// Inlined rather than served as a file.
///
/// One request, no cache to bust, and nothing to 404 — the page cannot
/// arrive unstyled because the styles cannot arrive separately. It is
/// small enough that this costs less than the extra round trip would.
const STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #f6f7f9;
  --card: #ffffff;
  --fg: #14161a;
  --muted: #5c6370;
  --line: #d8dce3;
  --accent: #2f6fed;
  --error: #b3261e;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0f1115;
    --card: #171a21;
    --fg: #e8eaed;
    --muted: #9aa1ad;
    --line: #2a2f3a;
    --accent: #6c9bff;
    --error: #f2b8b5;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 1.5rem;
  background: var(--bg);
  color: var(--fg);
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
}
.card {
  width: 100%;
  max-width: 22rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: 12px;
  padding: 2rem;
}
h1 { margin: 0 0 .25rem; font-size: 1.4rem; }
.sub { margin: 0 0 1.5rem; color: var(--muted); font-size: .9rem; }
label { display: block; margin: 0 0 .35rem; font-size: .85rem; font-weight: 600; }
input {
  width: 100%;
  margin: 0 0 1rem;
  padding: .6rem .7rem;
  font: inherit;
  color: var(--fg);
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 8px;
}
input:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
button {
  width: 100%;
  padding: .65rem;
  font: inherit;
  font-weight: 600;
  color: #fff;
  background: var(--accent);
  border: 0;
  border-radius: 8px;
  cursor: pointer;
}
button:hover { filter: brightness(1.07); }
.error {
  margin: 0 0 1rem;
  padding: .6rem .7rem;
  font-size: .875rem;
  color: var(--error);
  border: 1px solid var(--error);
  border-radius: 8px;
}
.alt { margin: 1.25rem 0 0; font-size: .875rem; color: var(--muted); text-align: center; }
a { color: var(--accent); }
"#;

#[cfg(test)]
mod tests {
    use super::{Screen, render, safe_return_to};

    /// The open-redirect fence. Each of these, forwarded verbatim in a
    /// `Location:`, sends a freshly-signed-in person to someone else's
    /// site from a link that started at the real login screen.
    #[test]
    fn only_same_origin_paths_survive() {
        assert_eq!(safe_return_to(Some("/oauth2/authorize?x=1")), "/oauth2/authorize?x=1");

        for hostile in [
            "//evil.example",
            "https://evil.example",
            "http://evil.example",
            "javascript:alert(1)",
            "/\\evil.example",
            "/ok\r\nLocation: https://evil.example",
        ] {
            assert_eq!(
                safe_return_to(Some(hostile)),
                "/",
                "{hostile} should not survive"
            );
        }
    }

    #[test]
    fn a_missing_return_to_is_the_default() {
        assert_eq!(safe_return_to(None), "/");
        assert_eq!(safe_return_to(Some("   ")), "/");
    }

    /// The form has to carry `return_to` forward, or the resumed
    /// authorize request is lost the moment someone signs in.
    #[test]
    fn the_form_carries_the_destination() {
        let html = render(Screen::SignIn, "/oauth2/authorize?client_id=task", None);
        assert!(html.contains(r#"name="return_to""#));
        assert!(html.contains("/oauth2/authorize?client_id=task"));
        assert!(html.contains(r#"action="/login""#));
    }

    #[test]
    fn an_error_is_shown_to_the_person() {
        let html = render(Screen::SignIn, "/", Some("Email or password is incorrect."));
        assert!(html.contains("Email or password is incorrect."));
        assert!(html.contains(r#"role="alert""#));
    }

    /// Password managers key off these; the wrong one is why they
    /// sometimes refuse to fill or offer to save.
    #[test]
    fn sign_up_and_sign_in_ask_for_different_passwords() {
        assert!(render(Screen::SignUp, "/", None).contains("new-password"));
        assert!(render(Screen::SignIn, "/", None).contains("current-password"));
    }

    #[test]
    fn each_screen_offers_the_other() {
        assert!(render(Screen::SignIn, "/", None).contains("/sign-up"));
        assert!(render(Screen::SignUp, "/", None).contains("/login"));
    }
}
