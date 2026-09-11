//! `/account/passkeys` — the credentials that replace a password.
//!
//! # The JSON endpoints
//!
//! Registration and sign-in each need two round trips with a browser
//! API in between, so these four routes speak JSON rather than form
//! posts. That is not a departure from the rest of these pages so much
//! as an admission: `navigator.credentials` cannot be reached from a
//! form, so a passkey needs script no matter how the transport is
//! shaped. Everything that *can* be a form still is — listing and
//! deleting work with no JavaScript at all.
//!
//! # What the browser is trusted with
//!
//! Nothing. The `credential` object it posts is passed straight to
//! `webauthn-rs`, which verifies it against the challenge the server
//! parked and the public key the server stored. The handle names which
//! ceremony; it does not authorise anything on its own.

use architect_auth::{
    AuthStorage, BeginPasskeyAuthentication, BeginPasskeyRegistration,
    CompletePasskeyAuthentication, CompletePasskeyRegistration, DeletePasskey, ListPasskeys,
};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::{Form, Json};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::UiState;
use crate::page::{Flash, flash_to, safe_path, sign_in_first, token_of};
use crate::passkey_script::PASSKEY_SCRIPT;
use crate::profile::{FlashLine, message};
use crate::settings::Nav;

const PATH: &str = "/account/passkeys";

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct DeleteForm {
    pub credential_id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct FinishRegistration {
    pub handle: String,
    #[serde(default)]
    pub name: String,
    pub credential: Value,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct StartSignIn {
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct FinishSignIn {
    pub handle: String,
    pub credential: Value,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PasskeyRow {
    pub credential_id: String,
    pub name: String,
    pub added: String,
    pub backed_up: bool,
}

/// `GET /account/passkeys`
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
    let Ok(passkeys) = state
        .auth
        .list_passkeys(ListPasskeys {
            session_token: token,
        })
        .await
    else {
        return sign_in_first(PATH);
    };
    let Some(nav) = Nav::build(&state, &headers, PATH).await else {
        return sign_in_first(PATH);
    };
    crate::settings::document_with_script(
        "Passkeys",
        "Passkeys",
        "Sign in with your fingerprint, face or screen lock instead of a password.",
        &nav,
        PASSKEY_SCRIPT,
        rsx! {
            PasskeysView {
                rows: passkeys
                    .into_iter()
                    .map(|passkey| PasskeyRow {
                        name: passkey.name,
                        added: passkey.created_at.format("%Y-%m-%d").to_string(),
                        backed_up: passkey.backed_up,
                        credential_id: passkey.credential_id,
                    })
                    .collect::<Vec<_>>(),
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /account/passkeys/begin` — the creation options.
pub async fn begin_registration<S>(State(state): State<UiState<S>>, headers: HeaderMap) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    match state
        .auth
        .begin_passkey_registration(BeginPasskeyRegistration {
            session_token: token,
        })
        .await
    {
        Ok(challenge) => Json(json!({
            "options": challenge.options_json,
            "handle": challenge.handle,
        }))
        .into_response(),
        Err(error) => refuse(StatusCode::BAD_REQUEST, &message(&error)),
    }
}

/// `POST /account/passkeys/complete`
pub async fn complete_registration<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Json(body): Json<FinishRegistration>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    match state
        .auth
        .complete_passkey_registration(CompletePasskeyRegistration {
            session_token: token,
            handle: body.handle,
            name: body.name,
            credential_json: body.credential.to_string(),
        })
        .await
    {
        Ok(passkey) => Json(json!({ "name": passkey.name })).into_response(),
        Err(error) => refuse(StatusCode::BAD_REQUEST, &message(&error)),
    }
}

/// `POST /account/passkeys/delete` — a plain form, so it needs no script.
pub async fn delete<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<DeleteForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .delete_passkey(DeletePasskey {
            session_token: token,
            credential_id: form.credential_id,
        })
        .await
    {
        Ok(()) => flash_to(PATH, &Flash::Ok("Passkey removed.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /login/passkey/begin`
pub async fn begin_sign_in<S>(
    State(state): State<UiState<S>>,
    Json(body): Json<StartSignIn>,
) -> Response
where
    S: AuthStorage,
{
    // Unauthenticated on purpose, and answered identically for every
    // address: see `BeginPasskeyAuthentication`.
    match state
        .auth
        .begin_passkey_authentication(BeginPasskeyAuthentication {
            email: body.email.filter(|email| !email.trim().is_empty()),
        })
        .await
    {
        Ok(challenge) => Json(json!({
            "options": challenge.options_json,
            "handle": challenge.handle,
        }))
        .into_response(),
        Err(error) => refuse(StatusCode::BAD_REQUEST, &message(&error)),
    }
}

/// `POST /login/passkey/complete` — sets the session cookie.
pub async fn complete_sign_in<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Json(body): Json<FinishSignIn>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_path(body.return_to.as_deref());
    match state
        .auth
        .complete_passkey_authentication(CompletePasskeyAuthentication {
            handle: body.handle,
            credential_json: body.credential.to_string(),
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await
    {
        Ok(bundle) => {
            // A passkey is a second factor by construction — the
            // authenticator verified the person before it would sign —
            // so the session it issues is active and there is no
            // challenge to route through.
            let cookie = state.cookie.session_cookie(bundle.token.clone());
            let mut response = (
                StatusCode::OK,
                [(header::SET_COOKIE, cookie.to_string())],
                Json(json!({ "redirect": return_to })),
            )
                .into_response();
            crate::login::remember_signed_in(&mut response, &state.cookie, &headers, &bundle);
            response
        }
        Err(error) => refuse(StatusCode::UNAUTHORIZED, &message(&error)),
    }
}

/// A JSON error the script can put in front of a person.
fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
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

/// The passkey sign-in button, for a host application's login page.
///
/// Hidden until the script confirms the browser has the API, so a
/// browser without it shows no button rather than a dead one.
#[component]
pub fn SignInButton(return_to: String) -> Element {
    rsx! {
        div { "data-passkey": "true", hidden: true, class: "passkey-signin",
            button {
                r#type: "button",
                id: "passkey-signin",
                class: "button",
                "data-return-to": "{return_to}",
                "Sign in with a passkey"
            }
            p { id: "passkey-status", hidden: true }
        }
    }
}

#[component]
fn PasskeysView(rows: Vec<PasskeyRow>, flash: Option<Flash>) -> Element {
    rsx! {
        FlashLine { flash }

        section { class: "panel",
        if rows.is_empty() {
            p { class: "hint", "No passkeys yet." }
        } else {
            div { class: "wide",
            table { class: "grid",
                thead {
                    tr {
                        th { "Name" }
                        th { "Added" }
                        th { "Synced" }
                        th { }
                    }
                }
                tbody {
                    for row in rows.iter() {
                        tr { key: "{row.credential_id}",
                            td { "{row.name}" }
                            td { class: "mono", "{row.added}" }
                            td {
                                if row.backed_up {
                                    span { class: "tag", "yes" }
                                } else {
                                    span { class: "handle", "this device only" }
                                }
                            }
                            td {
                                // A form, not script: removing one must
                                // work even where creating one cannot.
                                form { method: "post", action: "{PATH}/delete", class: "inline",
                                    input { r#type: "hidden", name: "credential_id", value: "{row.credential_id}" }
                                    button { r#type: "submit", class: "link danger", "Remove" }
                                }
                            }
                        }
                    }
                }
            }
            }
        }
        }

        // Everything below needs `navigator.credentials`, so it stays
        // hidden unless the script found it.
        section { class: "panel", "data-passkey": "true", hidden: true,
            h2 { "Add a passkey" }
            p { class: "hint",
                "Your device will ask you to confirm. The key never leaves it — this site only ever sees the public half."
            }
            div { class: "stack",
                label { r#for: "passkey-name", "Name" }
                input { id: "passkey-name", placeholder: "My phone" }
                button { r#type: "button", id: "passkey-register", "Add a passkey" }
            }
            p { id: "passkey-status", hidden: true }
        }

    }
}
