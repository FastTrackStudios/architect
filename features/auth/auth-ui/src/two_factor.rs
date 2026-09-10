//! Two-factor: enrolling in it, proving it at sign-in, turning it off.
//!
//! # Three pages, because there are three moments
//!
//! `/account/two-factor` is where somebody who is already signed in
//! turns it on or off. `/login/two-factor` is the challenge between
//! typing a password and being signed in — a different page because at
//! that moment there is no usable session to render an account page
//! with. And enrolment has its own step in between, where the secret is
//! shown, because it is shown exactly once.
//!
//! # Why the enrolment page is a page and not a modal
//!
//! What it shows cannot be recovered: the secret is stored encrypted
//! and the backup codes only as hashes. A person who navigates away
//! has to start again — which is safe, because enrolment is not
//! *enabled* until a code from the app confirms it, so leaving early
//! changes nothing about the account.
//!
//! The QR is drawn as inline SVG rather than fetched, so the page still
//! works with images blocked and prints correctly.

use architect_auth::{
    AuthStorage, BeginTwoFactorEnrollment, ConfirmTwoFactor, CurrentSession, DisableTwoFactor,
    VerifyTwoFactor,
};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Redirect, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::page::{Flash, document, flash_to, sign_in_first, token_of};
use crate::profile::{FlashLine, message};

const PATH: &str = "/account/two-factor";

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct ChallengeQuery {
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CodeForm {
    pub code: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

/// `GET /account/two-factor` — on or off, and the button to change it.
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
    document(
        "Two-factor authentication",
        rsx! {
            StatusView {
                enabled: session.user.two_factor_enabled,
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /account/two-factor/enroll` — mint a secret and show it once.
pub async fn enroll<S>(State(state): State<UiState<S>>, headers: HeaderMap) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    let Ok(session) = state
        .auth
        .current_session(CurrentSession {
            token: token.clone(),
        })
        .await
    else {
        return sign_in_first(PATH);
    };
    let label = session
        .user
        .email
        .clone()
        .unwrap_or_else(|| session.user.id.to_string());

    match state
        .auth
        .begin_two_factor_enrollment(BeginTwoFactorEnrollment {
            session_token: token,
            account_label: label,
            issuer: state.issuer.clone(),
        })
        .await
    {
        Ok(enrollment) => {
            // Rendered straight from the POST rather than redirected
            // to: this is the one page whose content cannot be fetched
            // again, so putting it behind a redirect would mean either
            // holding it in a session or passing a secret through a URL.
            let qr = crate::qr::svg(&enrollment.otpauth_url, "Scan with your authenticator app")
                .unwrap_or_default();
            document(
                "Set up two-factor",
                rsx! {
                    EnrollView {
                        qr,
                        secret: enrollment.secret,
                        otpauth_url: enrollment.otpauth_url,
                        backup_codes: enrollment.backup_codes,
                    }
                },
            )
        }
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /account/two-factor/confirm` — a code from the app switches it on.
pub async fn confirm<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .confirm_two_factor(ConfirmTwoFactor {
            session_token: token,
            code: form.code.trim().to_owned(),
        })
        .await
    {
        Ok(()) => flash_to(
            PATH,
            &Flash::Ok("Two-factor authentication is on. Keep your backup codes safe.".into()),
        ),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /account/two-factor/disable`
pub async fn disable<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .disable_two_factor(DisableTwoFactor {
            session_token: token,
            code: form.code.trim().to_owned(),
        })
        .await
    {
        Ok(()) => flash_to(PATH, &Flash::Ok("Two-factor authentication is off.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

// ── The sign-in challenge ────────────────────────────────────────────

/// `GET /login/two-factor` — between the password and being signed in.
///
/// Reached with a session cookie already set, for a session that is not
/// active yet. Every other page treats such a session as no session at
/// all, which is what makes this page necessary: without it, signing in
/// with two-factor on redirects to the account page, which bounces back
/// to sign-in, forever.
pub async fn challenge<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<ChallengeQuery>,
) -> Response
where
    S: AuthStorage,
{
    if token_of(&headers, &state.cookie).is_none() {
        return Redirect::to("/login").into_response();
    }
    document(
        "Two-factor",
        rsx! {
            ChallengeView {
                return_to: q.return_to.unwrap_or_else(|| "/".to_owned()),
                error: q.error,
            }
        },
    )
}

/// `POST /login/two-factor`
pub async fn verify<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return Redirect::to("/login").into_response();
    };
    let return_to = crate::page::safe_path(form.return_to.as_deref());
    match state
        .auth
        .verify_two_factor(VerifyTwoFactor {
            session_token: token,
            code: form.code.trim().to_owned(),
        })
        .await
    {
        Ok(()) => Redirect::to(&return_to).into_response(),
        Err(error) => {
            let encoded = architect_auth::percent::encode_component(&message(&error));
            let back = architect_auth::percent::encode_component(&return_to);
            Redirect::to(&format!(
                "/login/two-factor?return_to={back}&error={encoded}"
            ))
            .into_response()
        }
    }
}

// ── Views ────────────────────────────────────────────────────────────

#[component]
fn StatusView(enabled: bool, flash: Option<Flash>) -> Element {
    rsx! {
        h1 { "Two-factor authentication" }
        p { class: "sub",
            if enabled {
                "On. Signing in asks for a code from your authenticator app."
            } else {
                "Off. Anyone with your password can sign in as you."
            }
        }
        FlashLine { flash }

        if enabled {
            h2 { "Turn it off" }
            p { class: "hint",
                "Enter a code from your app, or one of your backup codes, to confirm it is you."
            }
            form { method: "post", action: "{PATH}/disable", class: "stack",
                label { r#for: "disable-code", "Code" }
                input {
                    id: "disable-code",
                    name: "code",
                    required: true,
                    autocomplete: "one-time-code",
                    inputmode: "numeric",
                }
                button { r#type: "submit", class: "danger", "Turn off two-factor" }
            }
        } else {
            form { method: "post", action: "{PATH}/enroll", class: "stack",
                p { class: "hint",
                    "You will scan a code with an authenticator app, then confirm it works. Nothing changes until you confirm."
                }
                button { r#type: "submit", "Set up two-factor" }
            }
        }

        p { class: "alt",
            a { href: "/account/profile", "Your profile" }
            " · "
            a { href: "/account/sessions", "Active sessions" }
        }
    }
}

#[component]
fn EnrollView(
    qr: String,
    secret: String,
    otpauth_url: String,
    backup_codes: Vec<String>,
) -> Element {
    rsx! {
        h1 { "Set up two-factor" }
        p { class: "sub", "Two steps. Nothing is switched on until the second one." }

        h2 { "1. Add it to your app" }
        div { class: "qr", dangerous_inner_html: "{qr}" }
        p { class: "hint", "No camera? Type this into your app instead:" }
        input { class: "mono", readonly: true, value: "{secret}", "aria-label": "Setup key" }
        p { class: "hint",
            // A tap on a phone opens the app directly with everything
            // filled in — worth having for the case where the phone IS
            // the device reading this page.
            a { href: "{otpauth_url}", "Open in your authenticator app" }
        }

        h2 { "2. Save your backup codes" }
        p { class: "hint",
            "Each works once, in place of a code from the app. They are shown now and never again — only their hashes are stored. Print them or put them in your password manager."
        }
        ul { class: "codes",
            for code in backup_codes.iter() {
                li { class: "mono", "{code}" }
            }
        }

        h2 { "3. Confirm" }
        p { class: "hint", "Enter the six-digit code your app is showing." }
        form { method: "post", action: "{PATH}/confirm", class: "stack",
            label { r#for: "confirm-code", "Code from your app" }
            input {
                id: "confirm-code",
                name: "code",
                required: true,
                autocomplete: "one-time-code",
                inputmode: "numeric",
                autofocus: true,
            }
            button { r#type: "submit", "Turn on two-factor" }
        }

        p { class: "alt", a { href: "{PATH}", "Cancel" } }
    }
}

#[component]
fn ChallengeView(return_to: String, error: Option<String>) -> Element {
    rsx! {
        h1 { "One more step" }
        p { class: "sub", "Enter the code from your authenticator app." }
        if let Some(message) = error {
            p { class: "error", role: "alert", "{message}" }
        }
        form { method: "post", action: "/login/two-factor", class: "stack",
            input { r#type: "hidden", name: "return_to", value: "{return_to}" }
            label { r#for: "code", "Code" }
            input {
                id: "code",
                name: "code",
                required: true,
                autocomplete: "one-time-code",
                inputmode: "numeric",
                autofocus: true,
            }
            button { r#type: "submit", "Continue" }
        }
        p { class: "hint",
            "Lost your phone? Use one of the backup codes you saved when you turned this on."
        }
        p { class: "alt",
            form { method: "post", action: "/auth/sign-out", class: "inline",
                button { r#type: "submit", class: "link", "Sign in as someone else" }
            }
        }
    }
}
