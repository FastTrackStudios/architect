//! Signing in with an Ethereum wallet.
//!
//! # The message comes from here, not the browser
//!
//! `personal_sign` will sign whatever it is handed, so whoever composes
//! the message decides what is being agreed to. The server composes it:
//! the domain line is what stops a signature collected on one site
//! being replayed on another, and a message assembled in the page could
//! be edited before it reached the wallet.
//!
//! # A wallet is a way in, not a way to be found
//!
//! `/login/wallet/begin` mints a nonce and returns a message; it says
//! nothing about whether the address has an account here, because it
//! has not been told an address yet. Signing is what identifies
//! somebody, and by then they have proved they hold the key.

use architect_auth::{AuthStorage, CreateSiweNonce, VerifySiweMessage};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use dioxus::prelude::*;
use serde_json::json;

use crate::UiState;
use crate::page::safe_path;
use crate::profile::message;

#[derive(Debug, serde::Deserialize)]
pub struct FinishSignIn {
    pub message: String,
    pub signature: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

/// `POST /login/wallet/begin` — a nonce, inside a message to sign.
pub async fn begin<S>(State(state): State<UiState<S>>) -> Response
where
    S: AuthStorage,
{
    match state.auth.create_siwe_nonce(CreateSiweNonce).await {
        Ok(nonce) => {
            // The shape `parse_siwe_message` reads: the domain on the
            // first line, then `Address:` and `Nonce:`. The address is
            // filled in by the browser, which is safe because the
            // signature is checked against whatever ends up there.
            let message = format!(
                "{domain}\nSign in to {domain}\n\nURI: {base}\nVersion: 1\nChain ID: 1\nNonce: {nonce}",
                domain = state.siwe_domain,
                base = state.base_url,
                nonce = nonce.token,
            );
            Json(json!({ "message": message, "nonce": nonce.token })).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": message(&error) })),
        )
            .into_response(),
    }
}

/// `POST /login/wallet/complete`
pub async fn complete<S>(
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
        .verify_siwe_message(VerifySiweMessage {
            message: body.message,
            signature: body.signature,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await
    {
        Ok(bundle) => {
            let cookie = state.cookie.session_cookie(bundle.token.clone());
            // A wallet is one factor. If the account owes a second, the
            // session it gets is inactive and the challenge is where
            // the browser has to go.
            let destination = if bundle.session.active {
                return_to
            } else {
                format!(
                    "/login/two-factor?return_to={}",
                    architect_auth::percent::encode_component(&return_to)
                )
            };
            let mut response = (
                StatusCode::OK,
                [(header::SET_COOKIE, cookie.to_string())],
                Json(json!({ "redirect": destination })),
            )
                .into_response();
            crate::login::remember_signed_in(&mut response, &state.cookie, &headers, &bundle);
            response
        }
        Err(error) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": message(&error) })),
        )
            .into_response(),
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

/// The wallet button, for a host's sign-in page.
///
/// Hidden twice over: by the attribute until the script runs, and by
/// the script until it has seen `window.ethereum`. Most people have no
/// wallet, and a button that opens nothing is worse than no button.
#[component]
pub fn SignInButton(return_to: String) -> Element {
    rsx! {
        div { hidden: true, class: "wallet-signin",
            button {
                r#type: "button",
                id: "wallet-signin",
                class: "button",
                "data-return-to": "{return_to}",
                "Sign in with a wallet"
            }
        }
    }
}
