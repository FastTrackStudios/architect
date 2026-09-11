//! `/account/switch` — the accounts this browser is holding.
//!
//! Switching promotes one of the tokens already in the roster; it does
//! not sign anybody in again. That is the whole value: a work account
//! and a personal one, one click apart, with neither logged out.
//!
//! # Signing out has two meanings here
//!
//! Leaving *this* account and leaving *all* of them are different acts,
//! and a page with several accounts on it has to offer both. "Sign out"
//! on a row ends that one session and drops it from the roster,
//! promoting whatever is left; "sign out of everything" ends all of
//! them. Offering only the first strands somebody who wanted to hand
//! the laptop back.

use architect_auth::{AuthStorage, ListDeviceSessions, SignOut};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::multi_session;
use crate::page::{Flash, flash_to, sign_in_first, token_of};
use crate::profile::FlashLine;
use crate::settings::Nav;

const PATH: &str = "/account/switch";

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TokenForm {
    pub token: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub token: String,
    pub name: String,
    pub email: String,
    pub current: bool,
}

/// `GET /account/switch`
pub async fn page<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response
where
    S: AuthStorage,
{
    let Some(current) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    // The current session may not be in the roster yet — it will not be
    // for anybody who signed in before this existed — so it is added
    // here rather than being absent from its own page.
    let mut tokens = multi_session::roster(&headers, &state.cookie.name);
    if !tokens.contains(&current) {
        tokens.insert(0, current.clone());
    }

    let Ok(sessions) = state
        .auth
        .list_device_sessions(ListDeviceSessions {
            session_tokens: tokens,
        })
        .await
    else {
        return sign_in_first(PATH);
    };

    let Some(nav) = Nav::build(&state, &headers, PATH).await else {
        return sign_in_first(PATH);
    };
    crate::settings::document(
        "Switch account",
        "Switch account",
        "Signed in on this browser. Switching does not sign anybody out.",
        &nav,
        rsx! {
            AccountsView {
                rows: sessions
                    .sessions
                    .into_iter()
                    .map(|session| AccountRow {
                        current: session.token == current,
                        name: session
                            .user
                            .name
                            .clone()
                            .filter(|n| !n.trim().is_empty())
                            .or_else(|| session.user.username.clone())
                            .unwrap_or_else(|| "Unnamed".to_owned()),
                        email: session.user.email.clone().unwrap_or_default(),
                        token: session.token,
                    })
                    .collect::<Vec<_>>(),
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /account/switch` — promote one of the held tokens.
pub async fn switch<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response
where
    S: AuthStorage,
{
    if token_of(&headers, &state.cookie).is_none() {
        return sign_in_first(PATH);
    }
    // Only a token this browser already holds. Without this the form is
    // "paste a session token here and become that person" — the roster
    // is the authorisation, since the browser was given each of these
    // by signing in.
    let held = multi_session::roster(&headers, &state.cookie.name);
    if !held.contains(&form.token) {
        return flash_to(
            PATH,
            &Flash::Error("That account is not signed in here.".into()),
        );
    }
    // And it must still resolve — a session that expired while sitting
    // in the roster must not become the active one.
    if state
        .auth
        .current_session(architect_auth::CurrentSession {
            token: form.token.clone(),
        })
        .await
        .is_err()
    {
        let mut response = flash_to(
            PATH,
            &Flash::Error("That session has expired. Sign in again.".into()),
        );
        multi_session::forget(&mut response, &state.cookie, &headers, &form.token);
        return response;
    }

    let cookie = state.cookie.session_cookie(form.token.clone());
    let mut response = (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie.to_string()),
            (header::LOCATION, PATH.to_owned()),
        ],
    )
        .into_response();
    multi_session::remember(&mut response, &state.cookie, &headers, &form.token);
    response
}

/// `POST /account/switch/leave` — sign out of one account.
pub async fn leave<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response
where
    S: AuthStorage,
{
    let held = multi_session::roster(&headers, &state.cookie.name);
    if !held.contains(&form.token) {
        return flash_to(
            PATH,
            &Flash::Error("That account is not signed in here.".into()),
        );
    }
    let _ = state
        .auth
        .sign_out(SignOut {
            token: form.token.clone(),
        })
        .await;

    let remaining: Vec<String> = held
        .into_iter()
        .filter(|token| *token != form.token)
        .collect();
    // Whatever is left becomes the active one, so leaving an account
    // does not sign you out of the others.
    let next = remaining.first().cloned();
    let mut response = next.as_ref().map_or_else(
        || {
            let cleared = state.cookie.session_cookie(String::new());
            (
                StatusCode::SEE_OTHER,
                [
                    (header::SET_COOKIE, cleared.to_string()),
                    (header::LOCATION, "/login".to_owned()),
                ],
            )
                .into_response()
        },
        |token| {
            let cookie = state.cookie.session_cookie(token.clone());
            (
                StatusCode::SEE_OTHER,
                [
                    (header::SET_COOKIE, cookie.to_string()),
                    (header::LOCATION, PATH.to_owned()),
                ],
            )
                .into_response()
        },
    );
    multi_session::forget(&mut response, &state.cookie, &headers, &form.token);
    response
}

/// `POST /account/switch/leave-all`
pub async fn leave_all<S>(State(state): State<UiState<S>>, headers: HeaderMap) -> Response
where
    S: AuthStorage,
{
    for token in multi_session::roster(&headers, &state.cookie.name) {
        let _ = state.auth.sign_out(SignOut { token }).await;
    }
    if let Some(current) = token_of(&headers, &state.cookie) {
        let _ = state.auth.sign_out(SignOut { token: current }).await;
    }
    let cleared = state.cookie.session_cookie(String::new());
    let mut response = (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cleared.to_string()),
            (header::LOCATION, "/login".to_owned()),
        ],
    )
        .into_response();
    multi_session::clear(&mut response, &state.cookie);
    response
}

#[component]
fn AccountsView(rows: Vec<AccountRow>, flash: Option<Flash>) -> Element {
    rsx! {
        FlashLine { flash }

        section { class: "panel",
        ul { class: "orgs",
            for row in rows.iter() {
                li { key: "{row.token}", class: "org",
                    span {
                        strong { "{row.name}" }
                        span { class: "handle", "{row.email}" }
                    }
                    if row.current {
                        span { class: "tag", "Current" }
                    } else {
                        form { method: "post", action: "{PATH}", class: "inline",
                            input { r#type: "hidden", name: "token", value: "{row.token}" }
                            button { r#type: "submit", class: "link", "Switch to this" }
                        }
                    }
                    form { method: "post", action: "{PATH}/leave", class: "inline",
                        input { r#type: "hidden", name: "token", value: "{row.token}" }
                        button { r#type: "submit", class: "link", "Sign out" }
                    }
                }
            }
        }

        p { class: "hint",
            "Sign in as somebody else and they will appear here too, up to five."
        }
        a { class: "button small", href: "/login?return_to=%2Faccount%2Fswitch", "Add another account" }
        }

        section { class: "panel",
            h2 { "Leaving the machine?" }
            p { class: "hint", "Ends every account above, not just this one." }
            form { method: "post", action: "{PATH}/leave-all", class: "stack",
                button { r#type: "submit", class: "danger", "Sign out of everything" }
            }
        }
    }
}
