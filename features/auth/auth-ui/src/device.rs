//! `/auth/device` — approving a sign-in that started somewhere else.
//!
//! A television, a CLI or a set-top box cannot open a browser, so it
//! shows a short code and asks the person to type it on a device that
//! can. This is that page.
//!
//! The path is not a choice: the engine puts `/auth/device` in the
//! `verification_uri` it hands the device, which is printed on a screen
//! somebody is reading. Serving it anywhere else would mean the
//! instructions point at a 404.
//!
//! # Why approval is a separate step from entering the code
//!
//! Because typing a code is not consent. The code names a client and a
//! scope, and the page has to *show* those before anybody agrees to
//! them — a code typed in from a screen across the room is exactly the
//! situation where somebody is not reading carefully, and the standard
//! attack is to read a code aloud to a stranger over the phone.
//!
//! So the flow is: enter a code, see who is asking and for what, then
//! approve or refuse. Refusing is offered as plainly as approving.

use architect_auth::{
    ApproveDeviceCode, AuthStorage, CurrentSession, DenyDeviceCode, VerifyDeviceCode,
};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::page::{document, sign_in_first, token_of};
use crate::profile::message;

const PATH: &str = "/auth/device";

#[derive(Debug, Default, serde::Deserialize)]
pub struct DeviceQuery {
    #[serde(default)]
    pub user_code: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CodeForm {
    pub user_code: String,
}

/// `GET /auth/device`
///
/// With a `user_code` in the URL — the `verification_uri_complete` a
/// device shows as a QR — it goes straight to the confirmation. Without
/// one it asks.
pub async fn page<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<DeviceQuery>,
) -> Response
where
    S: AuthStorage,
{
    // Signing in first, and coming back with the code still in hand.
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&return_here(q.user_code.as_deref()));
    };
    if state
        .auth
        .current_session(CurrentSession { token })
        .await
        .is_err()
    {
        return sign_in_first(&return_here(q.user_code.as_deref()));
    }

    let Some(user_code) = q
        .user_code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    else {
        return document("Connect a device", rsx! { AskView { error: q.error } });
    };

    match state
        .auth
        .verify_device_code(VerifyDeviceCode {
            user_code: normalize(user_code),
        })
        .await
    {
        Ok(verification) => document(
            "Connect a device",
            rsx! {
                ConfirmView {
                    user_code: verification.user_code,
                    client_id: verification.client_id,
                    scope: verification.scope.unwrap_or_default(),
                }
            },
        ),
        Err(error) => document(
            "Connect a device",
            rsx! { AskView { error: Some(message(&error)) } },
        ),
    }
}

/// `POST /auth/device` — look a typed code up.
pub async fn look_up<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response
where
    S: AuthStorage,
{
    let code = normalize(&form.user_code);
    if token_of(&headers, &state.cookie).is_none() {
        return sign_in_first(&return_here(Some(&code)));
    }
    // Straight back through the GET, so a refresh does not re-post and
    // the code sits in the URL where a person can see what they typed.
    axum::response::Redirect::to(&format!(
        "{PATH}?user_code={}",
        architect_auth::percent::encode_component(&code)
    ))
    .into_response()
}

/// `POST /auth/device/approve`
pub async fn approve<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response
where
    S: AuthStorage,
{
    let code = normalize(&form.user_code);
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(&return_here(Some(&code)));
    };
    match state
        .auth
        .approve_device_code(ApproveDeviceCode {
            session_token: token,
            user_code: code,
        })
        .await
    {
        Ok(()) => document("Device connected", rsx! { DoneView { approved: true } }),
        Err(error) => document(
            "Connect a device",
            rsx! { AskView { error: Some(message(&error)) } },
        ),
    }
}

/// `POST /auth/device/deny`
pub async fn deny<S>(State(state): State<UiState<S>>, Form(form): Form<CodeForm>) -> Response
where
    S: AuthStorage,
{
    // No session needed to refuse. Somebody who was read a code over
    // the phone and thought better of it should be able to shut it down
    // without first proving who they are.
    let _ = state
        .auth
        .deny_device_code(DenyDeviceCode {
            user_code: normalize(&form.user_code),
        })
        .await;
    document("Device refused", rsx! { DoneView { approved: false } })
}

/// Where to come back to after signing in.
fn return_here(user_code: Option<&str>) -> String {
    user_code.map_or_else(
        || PATH.to_owned(),
        |code| {
            format!(
                "{PATH}?user_code={}",
                architect_auth::percent::encode_component(code)
            )
        },
    )
}

/// A code as the person typed it, reduced to the form it was issued in.
///
/// Codes are shown grouped and uppercase — `WDJB-MJHT` — and read off a
/// screen across a room. So spaces, dashes and lowercase are all
/// expected input, and none of them should be a failure.
fn normalize(user_code: &str) -> String {
    user_code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_uppercase()
}

#[component]
fn AskView(error: Option<String>) -> Element {
    rsx! {
        h1 { "Connect a device" }
        p { class: "sub", "Enter the code shown on the other device." }
        if let Some(message) = error {
            p { class: "error", role: "alert", "{message}" }
        }
        form { method: "post", action: "{PATH}", class: "stack",
            label { r#for: "user_code", "Code" }
            input {
                id: "user_code",
                name: "user_code",
                required: true,
                autocapitalize: "characters",
                autocorrect: "off",
                spellcheck: "false",
                autofocus: true,
                placeholder: "WDJB-MJHT",
            }
            p { class: "hint", "Dashes, spaces and lowercase are all fine." }
            button { r#type: "submit", "Continue" }
        }
    }
}

#[component]
fn ConfirmView(user_code: String, client_id: String, scope: String) -> Element {
    rsx! {
        h1 { "Allow {client_id}?" }
        p { class: "sub", "It will be able to act as you." }

        table { class: "grid",
            tbody {
                tr {
                    td { "Application" }
                    td { class: "mono", "{client_id}" }
                }
                tr {
                    td { "Code" }
                    td { class: "mono", "{user_code}" }
                }
                if !scope.is_empty() {
                    tr {
                        td { "Access" }
                        td {
                            ul { class: "codes",
                                for item in scope.split_whitespace() {
                                    li { "{item}" }
                                }
                            }
                        }
                    }
                }
            }
        }

        p { class: "hint",
            "Only continue if you started this yourself, on a device you can see. Nobody should ever ask you for this code."
        }

        form { method: "post", action: "{PATH}/approve", class: "stack",
            input { r#type: "hidden", name: "user_code", value: "{user_code}" }
            button { r#type: "submit", "Allow" }
        }
        p { class: "alt",
            form { method: "post", action: "{PATH}/deny", class: "inline",
                input { r#type: "hidden", name: "user_code", value: "{user_code}" }
                button { r#type: "submit", class: "link danger", "No, refuse it" }
            }
        }
    }
}

#[component]
fn DoneView(approved: bool) -> Element {
    rsx! {
        if approved {
            h1 { "Device connected" }
            p { class: "sub", "You can go back to the other device now — it will finish on its own." }
        } else {
            h1 { "Device refused" }
            p { class: "sub", "Nothing was shared, and the code will not work." }
        }
        p { class: "alt", a { href: "/account/profile", "Your account" } }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn a_code_read_off_a_screen_survives_being_typed() {
        // All of these are the same code. A person copying from a
        // television gets the grouping and the case wrong, and none of
        // that should be a failure.
        for typed in [
            "WDJB-MJHT",
            "wdjb-mjht",
            "WDJB MJHT",
            " wdjbmjht ",
            "WDJB–MJHT",
        ] {
            assert_eq!(normalize(typed), "WDJBMJHT", "{typed:?}");
        }
    }

    #[test]
    fn nothing_typed_is_nothing() {
        assert_eq!(normalize("   "), "");
        assert_eq!(normalize("---"), "");
    }
}
