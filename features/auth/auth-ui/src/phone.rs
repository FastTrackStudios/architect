//! `/account/phone` — attaching a phone number to an account.
//!
//! Two steps, because a number nobody has proved they hold is worse
//! than no number: it is a recovery route to somebody else's phone.
//! So the number is stored only after a code sent to it comes back.
//!
//! # Not a sign-in route here
//!
//! `verify_phone_number_otp` can create a session, and this page never
//! asks it to. Signing in by phone means an unauthenticated endpoint
//! that sends an SMS to any number typed into it, which is a bill
//! somebody else pays. Adding a number to an account you are already
//! signed into has a session to rate-limit against; sign-in does not.
//! The engine supports both — this page picks the safe one.

use architect_auth::{AuthStorage, CurrentSession, SendPhoneNumberOtp, VerifyPhoneNumberOtp};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::page::{Flash, flash_to, sign_in_first, token_of};
use crate::profile::{FlashLine, message};
use crate::settings::Nav;

const PATH: &str = "/account/phone";

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub sent: Option<String>,
    #[serde(default)]
    pub phone_number: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct NumberForm {
    pub phone_number: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct CodeForm {
    pub phone_number: String,
    pub code: String,
}

/// `GET /account/phone`
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
    let Ok(session) = state.auth.current_session(CurrentSession { token }).await else {
        return sign_in_first(PATH);
    };
    let Some(nav) = Nav::build(&state, &headers, PATH).await else {
        return sign_in_first(PATH);
    };
    crate::settings::document(
        "Phone",
        "Phone",
        "A number we can reach you on, once you have proved you hold it.",
        &nav,
        rsx! {
            PhoneView {
                current: architect_auth::flows::phone_number::user_phone_number(&session.user)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                verified: architect_auth::flows::phone_number::user_phone_number_verified(
                    &session.user,
                )
                .unwrap_or(false),
                pending: q.phone_number.unwrap_or_default(),
                sent: q.sent.is_some(),
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /account/phone` — send a code to the number.
pub async fn send_code<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<NumberForm>,
) -> Response
where
    S: AuthStorage,
{
    if token_of(&headers, &state.cookie).is_none() {
        return sign_in_first(PATH);
    }
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    let number = form.phone_number.trim().to_owned();
    // Claimed first, unverified. `verify_phone_number_otp` attaches the
    // number to whichever account already holds it and **creates a new
    // one** when nobody does — so verifying before claiming would mint
    // a stray account and then refuse to give the number to this one.
    if let Err(error) = state
        .auth
        .update_phone_number(architect_auth::UpdatePhoneNumber {
            session_token: token,
            phone_number: number.clone(),
        })
        .await
    {
        return flash_to(PATH, &Flash::Error(message(&error)));
    }
    match state
        .auth
        .send_phone_number_otp(SendPhoneNumberOtp {
            phone_number: number.clone(),
        })
        .await
    {
        Ok(verification) => {
            state
                .sms
                .send_login_code(&number, &verification.token)
                .await;
            axum::response::Redirect::to(&format!(
                "{PATH}?sent=1&phone_number={}",
                architect_auth::percent::encode_component(&number)
            ))
            .into_response()
        }
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /account/phone/verify`
pub async fn verify<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response
where
    S: AuthStorage,
{
    if token_of(&headers, &state.cookie).is_none() {
        return sign_in_first(PATH);
    }
    match state
        .auth
        .verify_phone_number_otp(VerifyPhoneNumberOtp {
            phone_number: form.phone_number.trim().to_owned(),
            // Uppercased for the same reason the emailed code is:
            // generated codes are uppercase and a phone keyboard will
            // not capitalise.
            otp: form.code.trim().to_uppercase(),
            // Never a sign-in from this page. See the module docs.
            create_session: false,
            ip_address: None,
            user_agent: None,
        })
        .await
    {
        // The number was already attached when the code was sent; this
        // is what marks it confirmed.
        Ok(_) => flash_to(PATH, &Flash::Ok("Phone number confirmed.".into())),
        Err(_) => flash_to(
            PATH,
            &Flash::Error("That code is wrong or has expired. Ask for another.".into()),
        ),
    }
}

#[component]
fn PhoneView(
    current: String,
    verified: bool,
    pending: String,
    sent: bool,
    flash: Option<Flash>,
) -> Element {
    rsx! {
        section { class: "panel",
        if current.is_empty() {
            p { class: "sub", "No phone number on this account." }
        } else if verified {
            p { class: "sub", "Currently {current}, confirmed." }
        } else {
            // Saying nothing here would tell somebody a recovery route
            // works when it has never been tested.
            p { class: "sub", "Currently {current} — not confirmed yet." }
        }
        FlashLine { flash }

        if sent {
            p { class: "hint", "We sent a code to {pending}." }
            form { method: "post", action: "{PATH}/verify", class: "stack",
                input { r#type: "hidden", name: "phone_number", value: "{pending}" }
                label { r#for: "code", "Code" }
                input {
                    id: "code",
                    name: "code",
                    required: true,
                    autocomplete: "one-time-code",
                    autocapitalize: "characters",
                    autocorrect: "off",
                    spellcheck: "false",
                    autofocus: true,
                }
                button { r#type: "submit", "Save this number" }
            }
            form { method: "post", action: "{PATH}", class: "inline",
                input { r#type: "hidden", name: "phone_number", value: "{pending}" }
                button { r#type: "submit", class: "link", "Send another code" }
            }
        } else {
            form { method: "post", action: "{PATH}", class: "stack",
                label { r#for: "phone_number", "Number" }
                input {
                    id: "phone_number",
                    name: "phone_number",
                    r#type: "tel",
                    required: true,
                    autocomplete: "tel",
                    placeholder: "+44 7700 900000",
                    autofocus: true,
                }
                p { class: "hint",
                    "With the country code. We will send a code to check it reaches you."
                }
                button { r#type: "submit", "Send a code" }
            }
        }
        }

    }
}
