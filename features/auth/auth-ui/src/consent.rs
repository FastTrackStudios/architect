//! The screen that asks "do you want to let this app in?".
//!
//! `/oauth2/authorize` answers `VerificationRequired` when a client is
//! registered without `skip_consent` — meaning "ask them". Nothing
//! asked, so the request failed with an error instead, and a client
//! that wanted consent simply could not complete authorization.
//!
//! # What it must show
//!
//! The application and the access, in words, before the button. That
//! is the entire job of a consent screen and the reason a consent
//! screen exists: the redirect that follows is invisible, and this is
//! the only moment anybody sees what they are agreeing to.
//!
//! # Why refusing redirects back
//!
//! An OAuth client is waiting on a redirect, and a person who refuses
//! should not be left on a dead page while the application they came
//! from spins. The engine validated the `redirect_uri` on the way in,
//! so sending `error=access_denied` to it is exactly what the
//! specification asks for and lands them back where they started.

use architect_auth::{AuthStorage, AuthorizeOidc};
use axum::Form;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Redirect, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::page::{document, sign_in_first, token_of};
use crate::profile::message;

/// Everything `/oauth2/authorize` was called with, carried through the
/// consent form so the second attempt is the same request.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ConsentParams {
    pub client_id: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub response_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub code_challenge: Option<String>,
    #[serde(default)]
    pub code_challenge_method: Option<String>,
}

/// Render the consent screen for a request that needs one.
#[must_use]
pub fn page(params: &ConsentParams) -> Response {
    document(
        "Allow access",
        rsx! {
            ConsentView {
                client_id: params.client_id.clone(),
                scope: params.scope.clone().unwrap_or_default(),
                fields: params.clone(),
            }
        },
    )
}

/// `POST /oauth2/consent` — the answer.
pub async fn decide<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<ConsentForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first("/account/profile");
    };
    if form.decision.as_deref() != Some("allow") {
        // Back to the client with the refusal the specification names,
        // rather than stranding them here while it waits.
        return Redirect::to(&refusal_uri(&form.params)).into_response();
    }

    match state
        .auth
        .authorize_oidc(AuthorizeOidc {
            session_token: token,
            client_id: form.params.client_id.clone(),
            redirect_uri: form.params.redirect_uri.clone(),
            response_type: form
                .params
                .response_type
                .clone()
                .unwrap_or_else(|| "code".to_owned()),
            scope: form.params.scope.clone(),
            state: form.params.state.clone(),
            nonce: form.params.nonce.clone(),
            code_challenge: form.params.code_challenge.clone(),
            code_challenge_method: form.params.code_challenge_method.clone(),
            // The whole point of this round trip: consent has now been
            // given, so the request is re-issued without asking for it
            // again. Leaving `prompt` in place would loop forever.
            prompt: None,
        })
        .await
    {
        Ok(authorization) => Redirect::to(&authorization.redirect_uri).into_response(),
        Err(error) => document(
            "Allow access",
            rsx! {
                RefusedView { message: message(&error) }
            },
        ),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct ConsentForm {
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(flatten)]
    pub params: ConsentParams,
}

/// `redirect_uri` with `error=access_denied`, and the caller's `state`.
///
/// The `state` has to come back or the client cannot match the answer
/// to the request it made, which is the whole reason it sent one.
fn refusal_uri(params: &ConsentParams) -> String {
    let separator = if params.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    let mut uri = format!("{}{separator}error=access_denied", params.redirect_uri);
    if let Some(state) = params.state.as_deref().filter(|s| !s.is_empty()) {
        uri.push_str("&state=");
        uri.push_str(&architect_auth::percent::encode_component(state));
    }
    uri
}

/// A scope, in words.
///
/// Unknown scopes are shown as they are rather than hidden: a consent
/// screen that quietly omits what it does not recognise is asking
/// somebody to agree to something it declined to mention.
fn describe_scope(scope: &str) -> String {
    match scope {
        "openid" => "Confirm who you are".to_owned(),
        "profile" => "Your name and picture".to_owned(),
        "email" => "Your email address".to_owned(),
        "offline_access" => "Keep working when you are not here".to_owned(),
        other => other.to_owned(),
    }
}

#[component]
fn ConsentView(client_id: String, scope: String, fields: ConsentParams) -> Element {
    let scopes: Vec<String> = scope.split_whitespace().map(describe_scope).collect();
    rsx! {
        h1 { "Allow {client_id}?" }
        p { class: "sub", "It is asking to use your account." }

        if scopes.is_empty() {
            p { class: "hint", "No particular access was requested." }
        } else {
            h2 { "It will be able to" }
            ul { class: "codes",
                for item in scopes.iter() {
                    li { "{item}" }
                }
            }
        }

        p { class: "hint",
            "You can take this back at any time by signing out of that application."
        }

        form { method: "post", action: "/oauth2/consent", class: "stack",
            ConsentFields { fields: fields.clone() }
            input { r#type: "hidden", name: "decision", value: "allow" }
            button { r#type: "submit", "Allow" }
        }
        p { class: "alt",
            form { method: "post", action: "/oauth2/consent", class: "inline",
                ConsentFields { fields }
                input { r#type: "hidden", name: "decision", value: "deny" }
                button { r#type: "submit", class: "link danger", "No, refuse" }
            }
        }
    }
}

/// The original request, carried through the form.
///
/// Every field, because the second attempt has to be the *same*
/// request — a `nonce` or a `code_challenge` dropped here turns a
/// correct authorization into one the client will reject, and it would
/// look like a consent bug rather than a lost parameter.
#[component]
fn ConsentFields(fields: ConsentParams) -> Element {
    rsx! {
        input { r#type: "hidden", name: "client_id", value: "{fields.client_id}" }
        input { r#type: "hidden", name: "redirect_uri", value: "{fields.redirect_uri}" }
        if let Some(value) = fields.response_type.as_deref() {
            input { r#type: "hidden", name: "response_type", value: "{value}" }
        }
        if let Some(value) = fields.scope.as_deref() {
            input { r#type: "hidden", name: "scope", value: "{value}" }
        }
        if let Some(value) = fields.state.as_deref() {
            input { r#type: "hidden", name: "state", value: "{value}" }
        }
        if let Some(value) = fields.nonce.as_deref() {
            input { r#type: "hidden", name: "nonce", value: "{value}" }
        }
        if let Some(value) = fields.code_challenge.as_deref() {
            input { r#type: "hidden", name: "code_challenge", value: "{value}" }
        }
        if let Some(value) = fields.code_challenge_method.as_deref() {
            input { r#type: "hidden", name: "code_challenge_method", value: "{value}" }
        }
    }
}

#[component]
fn RefusedView(message: String) -> Element {
    rsx! {
        h1 { "That did not work" }
        p { class: "sub", "{message}" }
        p { class: "alt", a { href: "/account/profile", "Your account" } }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConsentParams, describe_scope, refusal_uri};

    fn params(redirect_uri: &str, state: Option<&str>) -> ConsentParams {
        ConsentParams {
            client_id: "task".into(),
            redirect_uri: redirect_uri.into(),
            state: state.map(str::to_owned),
            ..ConsentParams::default()
        }
    }

    #[test]
    fn refusing_goes_back_to_the_client_with_its_state() {
        // Without the state the client cannot match the answer to the
        // request it made, which is the only reason it sent one.
        assert_eq!(
            refusal_uri(&params("https://task.example/cb", Some("xyz"))),
            "https://task.example/cb?error=access_denied&state=xyz"
        );
    }

    #[test]
    fn a_redirect_uri_that_already_has_a_query_keeps_it() {
        assert_eq!(
            refusal_uri(&params("https://task.example/cb?a=1", None)),
            "https://task.example/cb?a=1&error=access_denied"
        );
    }

    #[test]
    fn a_state_that_carries_separators_cannot_break_the_url() {
        let uri = refusal_uri(&params("https://task.example/cb", Some("a&b=c")));
        assert!(uri.ends_with("state=a%26b%3Dc"), "{uri}");
    }

    #[test]
    fn scopes_are_named_and_unknown_ones_are_not_hidden() {
        assert_eq!(describe_scope("email"), "Your email address");
        // A consent screen that omits what it does not recognise is
        // asking somebody to agree to something it declined to mention.
        assert_eq!(describe_scope("forge:github"), "forge:github");
    }
}
