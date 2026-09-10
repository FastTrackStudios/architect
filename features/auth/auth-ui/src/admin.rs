//! `/admin/users` — the operator's screen.
//!
//! Every route here is behind [`architect_auth::AuthorizeAdmin`], and
//! the check is per-request rather than per-mount: an administrator who
//! is demoted mid-session stops being one on their next click, not at
//! their next sign-in.
//!
//! # What is deliberately blunt
//!
//! This is the smallest set of controls that resolves a support ticket:
//! find the person, see whether they are banned, change their role, ban
//! or unban, sign in as them, delete them. There is no bulk action and
//! no filter beyond paging, because every one of these is destructive
//! or privileged, and a screen that makes them fast makes mistakes
//! fast.
//!
//! # Impersonation
//!
//! It replaces the operator's own session cookie with one for the other
//! person, and the engine records who did it. Stopping returns to the
//! operator's own session. The banner is not decoration: without it, a
//! forgotten impersonation is an operator doing damage under somebody
//! else's name in every audit log downstream.

use architect_auth::{
    AuthStorage, AuthorizeAdmin, BanUser, CurrentSession, ImpersonateUser, ListUsers, RemoveUser,
    SetUserRole, StopImpersonating, UnbanUser,
};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use dioxus::prelude::*;
use uuid::Uuid;

use crate::UiState;
use crate::page::{Flash, document, flash_to, sign_in_first, token_of};
use crate::profile::{FlashLine, message};

const PATH: &str = "/admin/users";

/// How many people a page shows.
///
/// The engine clamps `limit` to 100; this is deliberately smaller,
/// because the list is read to find one person and a hundred rows is
/// not more findable than twenty-five.
const PAGE_SIZE: usize = 25;

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UserForm {
    pub user_id: Uuid,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub banned: bool,
    pub ban_reason: String,
    pub two_factor: bool,
    pub verified: bool,
    pub created: String,
    pub is_you: bool,
}

/// `GET /admin/users`
pub async fn users<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    // The gate is here rather than on the router so a demotion takes
    // effect on the next click.
    let Ok(admin) = state
        .auth
        .authorize_admin(AuthorizeAdmin {
            session_token: token.clone(),
        })
        .await
    else {
        return forbidden();
    };
    let offset = q.offset.unwrap_or(0);
    let Ok(listed) = state
        .auth
        .list_users(ListUsers {
            session_token: token.clone(),
            offset,
            limit: PAGE_SIZE,
        })
        .await
    else {
        return forbidden();
    };
    let session = state
        .auth
        .current_session(CurrentSession { token })
        .await
        .ok();

    document(
        "Users",
        rsx! {
            UsersView {
                rows: listed
                    .users
                    .into_iter()
                    .map(|user| UserRow {
                        is_you: user.id == admin.id,
                        email: user.email.clone().unwrap_or_default(),
                        name: user
                            .name
                            .clone()
                            .filter(|n| !n.trim().is_empty())
                            .or_else(|| user.username.clone())
                            .unwrap_or_default(),
                        role: user.role.clone().unwrap_or_else(|| "user".to_owned()),
                        banned: user.banned,
                        ban_reason: user.ban_reason.clone().unwrap_or_default(),
                        two_factor: user.two_factor_enabled,
                        verified: user.email_verified,
                        created: user.created_at.format("%Y-%m-%d").to_string(),
                        id: user.id,
                    })
                    .collect::<Vec<_>>(),
                total: listed.total,
                offset,
                impersonating: session
                    .and_then(|s| s.session.impersonated_by)
                    .is_some(),
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /admin/users/role`
pub async fn set_role<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<UserForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    let role = form.role.unwrap_or_default();
    let role = role.trim();
    match state
        .auth
        .set_user_role(SetUserRole {
            session_token: token,
            user_id: form.user_id,
            // Empty means "no special role", which the engine spells
            // `None`, not `Some("")`.
            role: (!role.is_empty() && role != "user").then(|| role.to_owned()),
        })
        .await
    {
        Ok(_) => flash_to(PATH, &Flash::Ok("Role updated.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /admin/users/ban`
pub async fn ban<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<UserForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    let reason = form.reason.unwrap_or_default();
    let reason = reason.trim();
    match state
        .auth
        .ban_user(BanUser {
            session_token: token,
            user_id: form.user_id,
            reason: (!reason.is_empty()).then(|| reason.to_owned()),
            expires_at: None,
        })
        .await
    {
        Ok(_) => flash_to(PATH, &Flash::Ok("Account banned.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /admin/users/unban`
pub async fn unban<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<UserForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .unban_user(UnbanUser {
            session_token: token,
            user_id: form.user_id,
        })
        .await
    {
        Ok(_) => flash_to(PATH, &Flash::Ok("Account unbanned.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /admin/users/delete`
pub async fn delete<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<UserForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .remove_user(RemoveUser {
            session_token: token,
            user_id: form.user_id,
        })
        .await
    {
        Ok(()) => flash_to(PATH, &Flash::Ok("Account deleted.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /admin/users/impersonate` — become them.
///
/// Swaps the cookie for a session belonging to the other person. The
/// engine records who did it, and [`stop`] puts it back.
pub async fn impersonate<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<UserForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .impersonate_user(ImpersonateUser {
            session_token: token.clone(),
            user_id: form.user_id,
            ip_address: None,
            user_agent: None,
        })
        .await
    {
        // The operator's own token is parked in a second cookie.
        // `impersonate_user` mints a NEW session for the target and
        // leaves the operator's alone, but the browser only has room
        // for one session cookie — so without this, stopping would
        // leave the operator holding a deactivated token and bounce
        // them to the login screen, which is a poor end to "have a
        // quick look at what they see".
        Ok(bundle) => {
            let mut response = swap_session(&state, &bundle.token, "/account/profile");
            let mut parked = state.cookie.session_cookie(token);
            parked.set_name(admin_cookie_name(&state.cookie.name));
            if let Ok(value) = parked.to_string().parse() {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            response
        }
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// The name of the cookie holding the operator's own session while they
/// are impersonating somebody.
fn admin_cookie_name(session_cookie: &str) -> String {
    format!("{session_cookie}.operator")
}

/// The parked operator token, if there is one.
fn parked_token(headers: &HeaderMap, session_cookie: &str) -> Option<String> {
    let name = admin_cookie_name(session_cookie);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, token)| token.to_owned())
}

/// `POST /admin/stop-impersonating`
pub async fn stop<S>(State(state): State<UiState<S>>, headers: HeaderMap) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    let operator = parked_token(&headers, &state.cookie.name);
    match state
        .auth
        .stop_impersonating(StopImpersonating {
            session_token: token,
        })
        .await
    {
        Ok(()) => {
            // Put the operator's own session back, and clear the park.
            // If there is nothing parked — a cookie dropped, or a
            // browser restarted — the impersonation session is still
            // deactivated, which is the important half; they just have
            // to sign in again.
            let mut response = operator.as_deref().map_or_else(
                || flash_to("/login", &Flash::Ok("Impersonation ended.".into())),
                |operator| swap_session(&state, operator, PATH),
            );
            // `Max-Age=0` is the only reliable way to ask a browser
            // to forget a cookie: an expiry in the past is compared
            // against a clock the browser owns. Built by hand because
            // the cookie crate's `Duration` is not re-exported.
            let cleared = format!(
                "{}=; Path={}; Max-Age=0; HttpOnly{}",
                admin_cookie_name(&state.cookie.name),
                state.cookie.path,
                if state.cookie.secure { "; Secure" } else { "" },
            );
            if let Ok(value) = cleared.parse() {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            response
        }
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// Replace the session cookie and go somewhere.
///
/// 303 so the browser switches to GET; a refresh on the destination
/// must not re-post the form that got there.
fn swap_session<S>(state: &UiState<S>, token: &str, to: &str) -> Response {
    let cookie = state.cookie.session_cookie(token.to_owned());
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie.to_string()),
            (header::LOCATION, to.to_owned()),
        ],
    )
        .into_response()
}

/// 403 as a page, not a redirect.
///
/// A signed-in person who is not an administrator has not failed to
/// sign in, so sending them to the login screen would be a lie that
/// loops. They are told plainly.
fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        document(
            "Not allowed",
            rsx! {
                h1 { "Not allowed" }
                p { class: "sub", "This page is for server administrators." }
                p { class: "alt", a { href: "/account/profile", "Your profile" } }
            },
        ),
    )
        .into_response()
}

/// The banner every page would show while impersonating, if the host
/// app renders it. Exposed so an embedder can use the same wording.
#[must_use]
pub const fn impersonation_notice() -> &'static str {
    "You are signed in as someone else."
}

#[component]
#[allow(clippy::fn_params_excessive_bools)]
fn UsersView(
    rows: Vec<UserRow>,
    total: usize,
    offset: usize,
    impersonating: bool,
    flash: Option<Flash>,
) -> Element {
    let shown = rows.len();
    let next = offset.saturating_add(PAGE_SIZE);
    let previous = offset.saturating_sub(PAGE_SIZE);
    rsx! {
        h1 { "Users" }
        p { class: "sub", "{total} account(s)." }
        FlashLine { flash }

        if impersonating {
            div { class: "minted",
                p { class: "error", role: "alert",
                    "You are signed in as someone else. Everything you do is recorded against them."
                }
                form { method: "post", action: "/admin/stop-impersonating", class: "inline",
                    button { r#type: "submit", "Stop impersonating" }
                }
            }
        }

        table { class: "grid",
            thead {
                tr {
                    th { "Person" }
                    th { "Role" }
                    th { "State" }
                    th { "Joined" }
                    th { }
                }
            }
            tbody {
                for row in rows.iter() {
                    tr { key: "{row.id}",
                        td {
                            if row.name.is_empty() {
                                span { class: "mono", "{row.email}" }
                            } else {
                                strong { "{row.name}" }
                                span { class: "handle", "{row.email}" }
                            }
                            if row.is_you {
                                span { class: "tag", "You" }
                            }
                        }
                        td {
                            form { method: "post", action: "{PATH}/role", class: "inline",
                                input { r#type: "hidden", name: "user_id", value: "{row.id}" }
                                select { name: "role", "aria-label": "Role for {row.email}",
                                    option { value: "user", selected: row.role != "admin", "User" }
                                    option { value: "admin", selected: row.role == "admin", "Admin" }
                                }
                                button { r#type: "submit", class: "link", "Set" }
                            }
                        }
                        td {
                            if row.banned {
                                span { class: "tag", "banned" }
                                if !row.ban_reason.is_empty() {
                                    span { class: "handle", "{row.ban_reason}" }
                                }
                            }
                            if row.two_factor {
                                span { class: "tag", "2FA" }
                            }
                            if !row.verified {
                                span { class: "tag", "unverified" }
                            }
                        }
                        td { class: "mono", "{row.created}" }
                        td {
                            if row.banned {
                                form { method: "post", action: "{PATH}/unban", class: "inline",
                                    input { r#type: "hidden", name: "user_id", value: "{row.id}" }
                                    button { r#type: "submit", class: "link", "Unban" }
                                }
                            } else if !row.is_you {
                                form { method: "post", action: "{PATH}/ban", class: "inline",
                                    input { r#type: "hidden", name: "user_id", value: "{row.id}" }
                                    input { name: "reason", placeholder: "reason", "aria-label": "Ban reason for {row.email}" }
                                    button { r#type: "submit", class: "link", "Ban" }
                                }
                            }
                            if !row.is_you {
                                form { method: "post", action: "{PATH}/impersonate", class: "inline",
                                    input { r#type: "hidden", name: "user_id", value: "{row.id}" }
                                    button { r#type: "submit", class: "link", "Sign in as" }
                                }
                                form { method: "post", action: "{PATH}/delete", class: "inline",
                                    input { r#type: "hidden", name: "user_id", value: "{row.id}" }
                                    button { r#type: "submit", class: "link danger", "Delete" }
                                }
                            }
                        }
                    }
                }
            }
        }

        p { class: "alt",
            if offset > 0 {
                a { href: "{PATH}?offset={previous}", "Previous" }
                " · "
            }
            if offset.saturating_add(shown) < total {
                a { href: "{PATH}?offset={next}", "Next" }
            }
        }
    }
}
