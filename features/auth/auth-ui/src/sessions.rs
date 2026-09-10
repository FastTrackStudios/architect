//! `/account/sessions` — where this account is signed in, and how to
//! stop it being signed in there.
//!
//! The page a person is sent to after "was that you?", so it has to
//! answer two questions on sight: which row is *this* browser, and how
//! do I end all the others in one action. Both are load-bearing — a
//! list that does not mark the current session invites somebody to
//! revoke the one they are reading it from, and a list without
//! "sign out everywhere" makes ending a compromise a clicking exercise.

use architect_auth::{
    AuthStorage, CurrentSession, ListSessions, RevokeOtherSessions, RevokeSession,
};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use dioxus::prelude::*;
use uuid::Uuid;

use crate::UiState;
use crate::page::{Flash, document, flash_to, sign_in_first, token_of};
use crate::profile::{FlashLine, message};

const PATH: &str = "/account/sessions";

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RevokeForm {
    pub session_id: Uuid,
}

/// One row of the table, already reduced to what the page shows.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: Uuid,
    pub current: bool,
    pub where_from: String,
    pub device: String,
    pub started: String,
    pub expires: String,
}

/// `GET /account/sessions`
pub async fn page<S>(
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
    let Ok(bundle) = state
        .auth
        .current_session(CurrentSession {
            token: token.clone(),
        })
        .await
    else {
        return sign_in_first(PATH);
    };
    let sessions = state
        .auth
        .list_sessions(ListSessions {
            session_token: token,
        })
        .await
        .unwrap_or_default();

    let rows: Vec<SessionRow> = sessions
        .into_iter()
        .map(|session| SessionRow {
            current: session.id == bundle.session.id,
            id: session.id,
            where_from: session
                .ip_address
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            device: describe_agent(session.user_agent.as_deref()),
            started: session.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
            expires: session.expires_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        })
        .collect();

    document(
        "Active sessions",
        rsx! {
            SessionsView {
                rows,
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /account/sessions/revoke`
pub async fn revoke<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<RevokeForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .revoke_session(RevokeSession {
            session_token: token,
            session_id: form.session_id,
        })
        .await
    {
        Ok(()) => flash_to(PATH, &Flash::Ok("Signed that session out.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /account/sessions/revoke-others`
pub async fn revoke_others<S>(State(state): State<UiState<S>>, headers: HeaderMap) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .revoke_other_sessions(RevokeOtherSessions {
            session_token: token,
        })
        .await
    {
        Ok(()) => flash_to(
            PATH,
            &Flash::Ok("Signed out everywhere except this browser.".into()),
        ),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// A user-agent string reduced to something a person recognises.
///
/// Not parsing: enough to tell one row from another when deciding which
/// to revoke. Order matters — every browser's UA claims to be several
/// other browsers, so the more specific names are tested first.
fn describe_agent(agent: Option<&str>) -> String {
    let Some(agent) = agent.filter(|a| !a.trim().is_empty()) else {
        return "Unknown device".to_owned();
    };
    let browser = [
        ("Edg/", "Edge"),
        ("OPR/", "Opera"),
        ("Firefox/", "Firefox"),
        ("Chrome/", "Chrome"),
        ("Safari/", "Safari"),
    ]
    .into_iter()
    .find_map(|(needle, name)| agent.contains(needle).then_some(name));
    let platform = [
        ("iPhone", "iPhone"),
        ("iPad", "iPad"),
        ("Android", "Android"),
        ("Mac OS X", "macOS"),
        ("Windows", "Windows"),
        ("Linux", "Linux"),
    ]
    .into_iter()
    .find_map(|(needle, name)| agent.contains(needle).then_some(name));
    match (browser, platform) {
        (Some(browser), Some(platform)) => format!("{browser} on {platform}"),
        (Some(browser), None) => browser.to_owned(),
        (None, Some(platform)) => platform.to_owned(),
        // Not a browser at all: a CLI or a service holding a session.
        (None, None) => agent.chars().take(40).collect(),
    }
}

#[component]
fn SessionsView(rows: Vec<SessionRow>, flash: Option<Flash>) -> Element {
    let others = rows.iter().filter(|row| !row.current).count();
    rsx! {
        h1 { "Active sessions" }
        p { class: "sub", "Everywhere this account is currently signed in." }
        FlashLine { flash }

        table { class: "grid",
            thead {
                tr {
                    th { "Device" }
                    th { "Address" }
                    th { "Started" }
                    th { "Expires" }
                    th { }
                }
            }
            tbody {
                for row in rows.iter() {
                    tr { key: "{row.id}",
                        td {
                            "{row.device}"
                            if row.current {
                                span { class: "tag", "This browser" }
                            }
                        }
                        td { class: "mono", "{row.where_from}" }
                        td { class: "mono", "{row.started}" }
                        td { class: "mono", "{row.expires}" }
                        td {
                            if row.current {
                                form { method: "post", action: "/auth/sign-out", class: "inline",
                                    button { r#type: "submit", class: "link", "Sign out" }
                                }
                            } else {
                                form { method: "post", action: "{PATH}/revoke", class: "inline",
                                    input { r#type: "hidden", name: "session_id", value: "{row.id}" }
                                    button { r#type: "submit", class: "link", "Revoke" }
                                }
                            }
                        }
                    }
                }
            }
        }

        if others > 0 {
            form { method: "post", action: "{PATH}/revoke-others", class: "stack",
                p { class: "hint",
                    "If you do not recognise a session above, sign out everywhere else and change your password."
                }
                button { r#type: "submit", "Sign out {others} other session(s)" }
            }
        }

        p { class: "alt",
            a { href: "/account/profile", "Your profile" }
            " · "
            a { href: "/orgs", "Organizations" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::describe_agent;

    #[test]
    fn a_browser_is_named_with_its_platform() {
        let chrome_on_mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                             (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
        assert_eq!(describe_agent(Some(chrome_on_mac)), "Chrome on macOS");
    }

    #[test]
    fn the_more_specific_browser_wins() {
        // Every one of these claims to be Chrome and Safari as well;
        // reporting "Safari" for an Edge session would make two rows
        // look like the same device.
        let edge = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like \
                    Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0";
        assert_eq!(describe_agent(Some(edge)), "Edge on Windows");
    }

    #[test]
    fn something_that_is_not_a_browser_shows_what_it_said() {
        assert_eq!(describe_agent(Some("task-cli/0.4.1")), "task-cli/0.4.1");
    }

    #[test]
    fn a_missing_agent_does_not_render_an_empty_cell() {
        assert_eq!(describe_agent(None), "Unknown device");
        assert_eq!(describe_agent(Some("   ")), "Unknown device");
    }
}
