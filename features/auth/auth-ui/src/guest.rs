//! Trying something before making an account, and keeping it afterwards.
//!
//! A guest is a real user with the role `anonymous` and no address. It
//! can own organizations, hold sessions and do work — which is the
//! point: somebody evaluating a product should be able to *use* it and
//! decide afterwards whether to keep what they made.
//!
//! # Upgrading, not migrating
//!
//! `link_anonymous_email_password` attaches credentials to the account
//! that already exists rather than creating a second one and copying
//! things across. So everything the guest made stays where it is, still
//! owned by the same id — no re-parenting, nothing to miss.
//!
//! # Why the warning is blunt
//!
//! A guest session is the only thing tying somebody to their work. Lose
//! the cookie and it is gone, with no address to recover it through and
//! nothing for support to look up. The page says exactly that, in those
//! words, because the failure is silent and total and people do not
//! expect it.

use architect_auth::{AuthStorage, CurrentSession, LinkAnonymousEmailPassword, SignInAnonymous};
use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse as _, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::page::{Flash, flash_to, safe_path, sign_in_first, token_of};
use crate::profile::message;

/// The role the engine gives a guest.
pub const ANONYMOUS_ROLE: &str = "anonymous";

/// Is this a guest account?
#[must_use]
pub fn is_guest(user: &architect_auth::proto::AuthUser) -> bool {
    user.role.as_deref() == Some(ANONYMOUS_ROLE)
}

#[derive(Debug, serde::Deserialize)]
pub struct GuestForm {
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpgradeForm {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub name: String,
}

/// `POST /login/guest` — start without an account.
pub async fn start<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<GuestForm>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_path(form.return_to.as_deref());
    match state
        .auth
        .sign_in_anonymous(SignInAnonymous {
            metadata_json: None,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await
    {
        Ok(bundle) => {
            let cookie = state.cookie.session_cookie(bundle.token.clone());
            let mut response = (
                axum::http::StatusCode::SEE_OTHER,
                [
                    (header::SET_COOKIE, cookie.to_string()),
                    (header::LOCATION, return_to),
                ],
            )
                .into_response();
            crate::login::remember_signed_in(&mut response, &state.cookie, &headers, &bundle);
            response
        }
        Err(error) => flash_to("/login", &Flash::Error(message(&error))),
    }
}

/// `POST /account/upgrade` — turn a guest into an account.
pub async fn upgrade<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<UpgradeForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first("/account/profile");
    };
    let name = form.name.trim();
    match state
        .auth
        .link_anonymous_email_password(LinkAnonymousEmailPassword {
            session_token: token,
            email: form.email.trim().to_owned(),
            password: form.password,
            name: (!name.is_empty()).then(|| name.to_owned()),
            username: None,
            image: None,
        })
        .await
    {
        // Upgrading issues a fresh session and ends the guest one, so
        // the cookie has to be replaced or the next click is signed out.
        Ok(bundle) => {
            let cookie = state.cookie.session_cookie(bundle.token.clone());
            let mut response = (
                axum::http::StatusCode::SEE_OTHER,
                [
                    (header::SET_COOKIE, cookie.to_string()),
                    (
                        header::LOCATION,
                        "/account/profile?ok=Your%20account%20is%20saved.".to_owned(),
                    ),
                ],
            )
                .into_response();
            crate::login::remember_signed_in(&mut response, &state.cookie, &headers, &bundle);
            response
        }
        Err(error) => flash_to("/account/profile", &Flash::Error(message(&error))),
    }
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

/// Whether the session on this request belongs to a guest.
pub async fn session_is_guest<S>(state: &UiState<S>, headers: &HeaderMap) -> bool
where
    S: AuthStorage,
{
    let Some(token) = token_of(headers, &state.cookie) else {
        return false;
    };
    state
        .auth
        .current_session(CurrentSession { token })
        .await
        .is_ok_and(|bundle| is_guest(&bundle.user))
}

/// The "save your account" panel, for a guest's profile page.
#[component]
pub fn UpgradePanel() -> Element {
    rsx! {
        h2 { "Save your account" }
        p { class: "error", role: "alert",
            "You are signed in as a guest. If you lose this browser session, everything you have made here is gone — there is no address to recover it through and nothing for anyone to look up."
        }
        p { class: "hint",
            "Adding an address keeps this same account, with everything already in it. Nothing is copied or moved."
        }
        form { method: "post", action: "/account/upgrade", class: "stack",
            label { r#for: "upgrade-name", "Name" }
            input { id: "upgrade-name", name: "name", autocomplete: "name" }

            label { r#for: "upgrade-email", "Email" }
            input {
                id: "upgrade-email",
                name: "email",
                r#type: "email",
                required: true,
                autocomplete: "email",
            }

            label { r#for: "upgrade-password", "Password" }
            input {
                id: "upgrade-password",
                name: "password",
                r#type: "password",
                required: true,
                minlength: "8",
                autocomplete: "new-password",
            }

            button { r#type: "submit", "Save my account" }
        }
    }
}

/// The "continue as a guest" button, for a host's sign-in page.
#[component]
pub fn GuestButton(return_to: String) -> Element {
    rsx! {
        form { method: "post", action: "/login/guest", class: "inline",
            input { r#type: "hidden", name: "return_to", value: "{return_to}" }
            button { r#type: "submit", class: "link", "Continue as a guest" }
        }
    }
}
