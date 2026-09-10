//! `/account/api-keys` — keys for programs acting as you.
//!
//! # Shown once
//!
//! Only `hash_token(secret, key)` is stored, so the key itself exists
//! for exactly one render. That shapes the page: creation renders its
//! own result rather than redirecting, because a redirect would mean
//! either parking a live credential in a session or putting it in a URL
//! — and a URL reaches every proxy log between here and the browser.
//!
//! What survives is the `prefix`, the first twelve characters. Enough
//! to tell two keys apart in a list and to recognise one in a log,
//! useless for authenticating.
//!
//! # Revoke and delete are different buttons
//!
//! Revoking disables a key and keeps the row, so it still appears in
//! the list and in an audit trail: "this key existed, and was turned
//! off". Deleting removes it. Somebody responding to a leak wants the
//! first; somebody tidying up wants the second. Offering only delete
//! would quietly destroy the evidence at the moment it matters most.

use architect_auth::{
    AuthStorage, CreateApiKey, CurrentSession, DeleteApiKey, ListApiKeys, RevokeApiKey,
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

const PATH: &str = "/account/api-keys";

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateForm {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub expires_days: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct IdForm {
    pub id: Uuid,
}

#[derive(Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub enabled: bool,
    pub expires: String,
    pub created: String,
}

/// `GET /account/api-keys`
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
    let Ok(keys) = state
        .auth
        .list_api_keys(ListApiKeys {
            session_token: token,
        })
        .await
    else {
        return sign_in_first(PATH);
    };
    document(
        "API keys",
        rsx! {
            KeysView {
                rows: rows(keys),
                minted: None,
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /account/api-keys` — mint one and show it, once.
pub async fn create<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CreateForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    let name = form.name.trim();
    // Blank means "does not expire", which is a real choice for a key
    // in a deployment that is not rotated on a schedule.
    let expires_at = form
        .expires_days
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|days| *days > 0)
        .map(architect_auth::expiry::in_days);

    match state
        .auth
        .create_api_key(CreateApiKey {
            session_token: token.clone(),
            name: (!name.is_empty()).then(|| name.to_owned()),
            expires_at,
            permissions_json: None,
            rate_limit_time_window: None,
            rate_limit_max: None,
            metadata_json: None,
        })
        .await
    {
        Ok(bundle) => {
            let keys = state
                .auth
                .list_api_keys(ListApiKeys {
                    session_token: token,
                })
                .await
                .unwrap_or_default();
            document(
                "API keys",
                rsx! {
                    KeysView {
                        rows: rows(keys),
                        minted: Some(bundle.key),
                        flash: Some(Flash::Ok("Key created.".into())),
                    }
                },
            )
        }
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /account/api-keys/revoke` — turn it off, keep the row.
pub async fn revoke<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<IdForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .revoke_api_key(RevokeApiKey {
            session_token: token,
            api_key_id: form.id,
        })
        .await
    {
        Ok(()) => flash_to(
            PATH,
            &Flash::Ok("Key revoked. It will no longer authenticate.".into()),
        ),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /account/api-keys/delete` — remove the row entirely.
pub async fn delete<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<IdForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .delete_api_key(DeleteApiKey {
            session_token: token,
            api_key_id: form.id,
        })
        .await
    {
        Ok(()) => flash_to(PATH, &Flash::Ok("Key deleted.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

fn rows(keys: Vec<architect_auth::proto::AuthApiKey>) -> Vec<KeyRow> {
    let mut rows: Vec<KeyRow> = keys
        .into_iter()
        .map(|key| KeyRow {
            name: key
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Unnamed key".to_owned()),
            prefix: key.prefix.clone().unwrap_or_default(),
            enabled: key.enabled,
            expires: key.expires_at.map_or_else(
                || "never".to_owned(),
                |at| at.format("%Y-%m-%d").to_string(),
            ),
            created: key.created_at.format("%Y-%m-%d").to_string(),
            id: key.id,
        })
        .collect();
    rows.sort_by(|a, b| b.created.cmp(&a.created));
    rows
}

/// The unused reader for a session, kept for symmetry with the other
/// pages that need the user rather than just the token.
#[allow(dead_code)]
async fn current<S>(state: &UiState<S>, headers: &HeaderMap) -> bool
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
        .is_ok()
}

#[component]
fn KeysView(rows: Vec<KeyRow>, minted: Option<String>, flash: Option<Flash>) -> Element {
    rsx! {
        h1 { "API keys" }
        p { class: "sub", "For programs that act as you — scripts, CI, the CLI." }
        FlashLine { flash }

        if let Some(key) = minted {
            div { class: "minted",
                p { class: "ok", role: "status",
                    "Copy this now. Only a hash is stored, so it cannot be shown again."
                }
                input { class: "mono", readonly: true, value: "{key}", "aria-label": "New API key" }
            }
        }

        if rows.is_empty() {
            p { class: "hint", "No keys yet." }
        } else {
            table { class: "grid",
                thead {
                    tr {
                        th { "Name" }
                        th { "Key" }
                        th { "Created" }
                        th { "Expires" }
                        th { }
                    }
                }
                tbody {
                    for row in rows.iter() {
                        tr { key: "{row.id}",
                            td {
                                "{row.name}"
                                if !row.enabled {
                                    span { class: "tag", "revoked" }
                                }
                            }
                            td { class: "mono", "{row.prefix}…" }
                            td { class: "mono", "{row.created}" }
                            td { class: "mono", "{row.expires}" }
                            td {
                                if row.enabled {
                                    form { method: "post", action: "{PATH}/revoke", class: "inline",
                                        input { r#type: "hidden", name: "id", value: "{row.id}" }
                                        button { r#type: "submit", class: "link", "Revoke" }
                                    }
                                }
                                form { method: "post", action: "{PATH}/delete", class: "inline",
                                    input { r#type: "hidden", name: "id", value: "{row.id}" }
                                    button { r#type: "submit", class: "link danger", "Delete" }
                                }
                            }
                        }
                    }
                }
            }
            p { class: "hint",
                "Revoking stops a key working and keeps the record. Deleting removes the record too — if you are responding to a leak, revoke."
            }
        }

        h2 { "New key" }
        form { method: "post", action: "{PATH}", class: "stack",
            label { r#for: "key-name", "Name" }
            input { id: "key-name", name: "name", placeholder: "CI deploys" }
            p { class: "hint", "So you know which one to revoke later." }

            label { r#for: "key-days", "Expires after (days)" }
            input { id: "key-days", name: "expires_days", r#type: "number", min: "1", placeholder: "never" }

            button { r#type: "submit", "Create key" }
        }

        p { class: "alt",
            a { href: "/account/profile", "Your profile" }
            " · "
            a { href: "/account/two-factor", "Two-factor" }
        }
    }
}
