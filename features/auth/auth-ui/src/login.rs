//! Signing in without a password: a code, or a link.
//!
//! # Why both
//!
//! They fail differently, and people reach for different ones. A link
//! is one tap and is the better experience when mail arrives on the
//! same device. A code can be carried between devices — read the mail
//! on a phone, type six digits into a television — and survives a mail
//! client that rewrites or pre-fetches links, which is a real thing
//! corporate scanners do and which silently spends a one-shot link.
//!
//! # Everything answers the same
//!
//! Neither route ever says whether an address has an account. Sending
//! is answered with "check your inbox" for every well-formed address,
//! delivery failures included; verification failures are one message
//! that does not distinguish "wrong code" from "no such account". The
//! sign-in form has always refused to distinguish "no such account"
//! from "wrong password" for the same reason, and a passwordless route
//! that leaked it would simply move the oracle.
//!
//! The one exception is deliberate: asking for a *second* code too
//! quickly is refused as a rate limit rather than silently swallowed,
//! because that answer does not depend on whether the account exists —
//! only on whether this address was asked about recently.

use architect_auth::transport::AuthCookieConfig;
use architect_auth::{
    AuthSessionBundle, AuthStorage, SendEmailOtp, SendMagicLink, VerifyEmailOtp, VerifyMagicLink,
};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::page::{document, safe_path};

/// Where a magic link lands. Must be a path this router serves.
const MAGIC_CALLBACK: &str = "/login/magic";

#[derive(Debug, Default, serde::Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub sent: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct MagicQuery {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct SendForm {
    pub email: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CodeForm {
    pub email: String,
    pub code: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

// ── A code ───────────────────────────────────────────────────────────

/// `GET /login/code` — ask for a code, or type one in.
pub async fn code_page<S>(State(_state): State<UiState<S>>, Query(q): Query<LoginQuery>) -> Response
where
    S: AuthStorage,
{
    let sent = q.sent.is_some();
    document(
        "Sign in with a code",
        rsx! {
            CodeView {
                email: q.email.unwrap_or_default(),
                return_to: safe_path(q.return_to.as_deref()),
                sent,
                error: q.error,
            }
        },
    )
}

/// `POST /login/code` — mail a code.
pub async fn send_code<S>(State(state): State<UiState<S>>, Form(form): Form<SendForm>) -> Response
where
    S: AuthStorage,
{
    let email = form.email.trim().to_owned();
    let return_to = safe_path(form.return_to.as_deref());
    match state
        .auth
        .send_email_otp(SendEmailOtp {
            email: email.clone(),
        })
        .await
    {
        Ok(verification) => {
            state
                .mailer
                .send_login_code(&email, &verification.token)
                .await;
            sent_to("/login/code", &email, &return_to, None)
        }
        // Asked again too soon. Surfaced rather than swallowed: the
        // answer depends on when this address was last asked about,
        // not on whether it has an account, so it tells a stranger
        // nothing they did not just do themselves.
        Err(architect_auth::proto::AuthFlowError::PermissionDenied) => sent_to(
            "/login/code",
            &email,
            &return_to,
            Some("A code was sent recently. Check your inbox, or wait a moment and try again."),
        ),
        // Everything else — a malformed address, a storage failure —
        // looks exactly like success. A sign-in page that answers
        // differently for addresses it knows is an account oracle.
        Err(_) => sent_to("/login/code", &email, &return_to, None),
    }
}

/// `POST /login/code/verify`
pub async fn verify_code<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response
where
    S: AuthStorage,
{
    let email = form.email.trim().to_owned();
    let return_to = safe_path(form.return_to.as_deref());
    let verified = state
        .auth
        .verify_email_otp(VerifyEmailOtp {
            email: email.clone(),
            // Uppercased before it is checked: generated codes are
            // uppercase, so a phone keyboard that does not capitalise
            // would otherwise turn a correctly-read code into a wrong
            // one. This can only rescue a right answer — every code the
            // engine mints is uppercase, so no wrong code becomes right.
            otp: form.code.trim().to_uppercase(),
            create_session: true,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await;

    match verified {
        Ok(result) => match (result.session, result.token) {
            (Some(session), Some(token)) => signed_in(
                &state.cookie,
                &AuthSessionBundle {
                    user: result.user,
                    session,
                    token,
                },
                &return_to,
            ),
            // A verified code that produced no session is a
            // configuration this page cannot finish; sending somebody
            // back to a form that will do the same thing is kinder than
            // a blank page.
            _ => back_to_code(&email, &return_to, "That code could not sign you in."),
        },
        Err(_) => back_to_code(
            &email,
            &return_to,
            "That code is wrong or has expired. Ask for another.",
        ),
    }
}

// ── A link ───────────────────────────────────────────────────────────

/// `GET /login/link`
pub async fn link_page<S>(State(_state): State<UiState<S>>, Query(q): Query<LoginQuery>) -> Response
where
    S: AuthStorage,
{
    let sent = q.sent.is_some();
    document(
        "Sign in with a link",
        rsx! {
            LinkView {
                email: q.email.unwrap_or_default(),
                return_to: safe_path(q.return_to.as_deref()),
                sent,
                error: q.error,
            }
        },
    )
}

/// `POST /login/link` — mail a one-click link.
pub async fn send_link<S>(State(state): State<UiState<S>>, Form(form): Form<SendForm>) -> Response
where
    S: AuthStorage,
{
    let email = form.email.trim().to_owned();
    let return_to = safe_path(form.return_to.as_deref());
    // The callback has to be a path this router serves, and the engine
    // only accepts one under its own base URL.
    let callback = format!("{}{MAGIC_CALLBACK}", state.base_url.trim_end_matches('/'));
    match state
        .auth
        .send_magic_link(SendMagicLink {
            email: email.clone(),
            callback_url: Some(callback),
        })
        .await
    {
        Ok(link) => {
            // `return_to` rides on the link rather than in the session,
            // so opening it in a different browser still lands where
            // the person started.
            let url = format!(
                "{}&return_to={}",
                link.url,
                architect_auth::percent::encode_component(&return_to)
            );
            state.mailer.send_magic_link(&email, &url).await;
            sent_to("/login/link", &email, &return_to, None)
        }
        Err(architect_auth::proto::AuthFlowError::PermissionDenied) => sent_to(
            "/login/link",
            &email,
            &return_to,
            Some("A link was sent recently. Check your inbox, or wait a moment and try again."),
        ),
        Err(_) => sent_to("/login/link", &email, &return_to, None),
    }
}

/// `GET /login/magic` — where the link lands.
pub async fn magic_callback<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<MagicQuery>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_path(q.return_to.as_deref());
    let (Some(email), Some(token)) = (q.email, q.token) else {
        return expired(&return_to);
    };
    match state
        .auth
        .verify_magic_link(VerifyMagicLink {
            email,
            token,
            callback_url: None,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await
    {
        Ok(verified) => signed_in(
            &state.cookie,
            &AuthSessionBundle {
                user: verified.user,
                session: verified.session,
                token: verified.token,
            },
            &return_to,
        ),
        // One page for every failure: used already, expired, tampered
        // with, or never valid. Naming which would tell somebody
        // holding a guessed token that it had once been real.
        Err(_) => expired(&return_to),
    }
}

// ── Shared ───────────────────────────────────────────────────────────

/// Redirect to the "we sent it" state of a page.
fn sent_to(path: &str, email: &str, return_to: &str, note: Option<&str>) -> Response {
    let mut url = format!(
        "{path}?sent=1&email={}&return_to={}",
        architect_auth::percent::encode_component(email),
        architect_auth::percent::encode_component(return_to),
    );
    if let Some(note) = note {
        url.push_str("&error=");
        url.push_str(&architect_auth::percent::encode_component(note));
    }
    axum::response::Redirect::to(&url).into_response()
}

fn back_to_code(email: &str, return_to: &str, message: &str) -> Response {
    sent_to("/login/code", email, return_to, Some(message))
}

/// Set the session cookie and go on.
///
/// 303 so the browser switches to GET: a refresh on the destination
/// must not re-submit the code.
fn signed_in(cookie: &AuthCookieConfig, bundle: &AuthSessionBundle, return_to: &str) -> Response {
    let set_cookie = cookie.session_cookie(bundle.token.clone());
    // A passwordless sign-in is still subject to two-factor, and the
    // session it issues is inactive until that is given — the same
    // rule the password form follows.
    let location = if bundle.session.active {
        return_to.to_owned()
    } else {
        format!(
            "/login/two-factor?return_to={}",
            architect_auth::percent::encode_component(return_to)
        )
    };
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, set_cookie.to_string()),
            (header::LOCATION, location),
        ],
    )
        .into_response()
}

fn expired(return_to: &str) -> Response {
    document(
        "Link expired",
        rsx! {
            ExpiredView { return_to: return_to.to_owned() }
        },
    )
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

// ── Views ────────────────────────────────────────────────────────────

#[component]
fn CodeView(email: String, return_to: String, sent: bool, error: Option<String>) -> Element {
    rsx! {
        h1 { "Sign in with a code" }
        if sent {
            p { class: "sub", "If {email} has an account, a code is on its way." }
        } else {
            p { class: "sub", "We will email you a short code to type back in." }
        }
        if let Some(message) = error {
            p { class: "error", role: "alert", "{message}" }
        }

        if sent {
            form { method: "post", action: "/login/code/verify", class: "stack",
                input { r#type: "hidden", name: "email", value: "{email}" }
                input { r#type: "hidden", name: "return_to", value: "{return_to}" }
                label { r#for: "code", "Code" }
                input {
                    id: "code",
                    name: "code",
                    required: true,
                    autocomplete: "one-time-code",
                    // Deliberately NOT `inputmode: numeric`. The code is
                    // uppercase base64url — letters, digits, `-` and `_`
                    // — and a numeric keypad cannot type any of that. It
                    // was numeric here until a test printed a real code.
                    autocapitalize: "characters",
                    autocorrect: "off",
                    spellcheck: "false",
                    autofocus: true,
                }
                p { class: "hint", "Six characters, from the email we just sent." }
                button { r#type: "submit", "Sign in" }
            }
            form { method: "post", action: "/login/code", class: "inline",
                input { r#type: "hidden", name: "email", value: "{email}" }
                input { r#type: "hidden", name: "return_to", value: "{return_to}" }
                button { r#type: "submit", class: "link", "Send another code" }
            }
        } else {
            form { method: "post", action: "/login/code", class: "stack",
                input { r#type: "hidden", name: "return_to", value: "{return_to}" }
                label { r#for: "email", "Email" }
                input {
                    id: "email",
                    name: "email",
                    r#type: "email",
                    required: true,
                    autocomplete: "email",
                    autofocus: true,
                    value: "{email}",
                }
                button { r#type: "submit", "Email me a code" }
            }
        }

        p { class: "alt",
            a { href: "/login", "Sign in with a password" }
            " · "
            a { href: "/login/link", "Email me a link instead" }
        }
    }
}

#[component]
fn LinkView(email: String, return_to: String, sent: bool, error: Option<String>) -> Element {
    rsx! {
        h1 { "Sign in with a link" }
        if sent {
            p { class: "sub", "If {email} has an account, a sign-in link is on its way." }
            p { class: "hint",
                "The link works once and expires. You can close this page — open the link on whichever device is easiest."
            }
        } else {
            p { class: "sub", "We will email you a link that signs you in." }
        }
        if let Some(message) = error {
            p { class: "error", role: "alert", "{message}" }
        }

        if sent {
            form { method: "post", action: "/login/link", class: "inline",
                input { r#type: "hidden", name: "email", value: "{email}" }
                input { r#type: "hidden", name: "return_to", value: "{return_to}" }
                button { r#type: "submit", class: "link", "Send another link" }
            }
        } else {
            form { method: "post", action: "/login/link", class: "stack",
                input { r#type: "hidden", name: "return_to", value: "{return_to}" }
                label { r#for: "email", "Email" }
                input {
                    id: "email",
                    name: "email",
                    r#type: "email",
                    required: true,
                    autocomplete: "email",
                    autofocus: true,
                    value: "{email}",
                }
                button { r#type: "submit", "Email me a link" }
            }
        }

        p { class: "alt",
            a { href: "/login", "Sign in with a password" }
            " · "
            a { href: "/login/code", "Email me a code instead" }
        }
    }
}

#[component]
fn ExpiredView(return_to: String) -> Element {
    rsx! {
        h1 { "This link is not usable" }
        p { class: "sub",
            "Sign-in links work once and expire. It may also have been opened already — some mail scanners follow links before you do."
        }
        form { method: "post", action: "/login/link", class: "stack",
            input { r#type: "hidden", name: "return_to", value: "{return_to}" }
            label { r#for: "email", "Email" }
            input { id: "email", name: "email", r#type: "email", required: true, autocomplete: "email" }
            button { r#type: "submit", "Send a new link" }
        }
        p { class: "alt", a { href: "/login", "Sign in with a password" } }
    }
}
