//! `/account/profile` — the page a person edits themselves on.
//!
//! Three forms rather than one, because they carry different risk and
//! different failure. Name and avatar are a plain save. Changing an
//! address may need the new one verified before it takes effect, so its
//! outcome is "check your mail", not "saved". Changing a password
//! requires the current one, and getting that wrong must not silently
//! discard the display name typed in a different form on the same page.

use architect_auth::{
    AuthStorage, ChangeEmail, ChangePassword, CurrentSession, UpdateProfile, UpdateUsername,
};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use dioxus::prelude::*;

use crate::UiState;
use crate::page::{Flash, document, flash_to, sign_in_first, token_of};

const PATH: &str = "/account/profile";

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ProfileForm {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub image: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct EmailForm {
    pub new_email: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct PasswordForm {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
}

/// `GET /account/profile`
pub async fn page<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response
where
    S: AuthStorage,
{
    let Some(session) = current(&state, &headers).await else {
        return sign_in_first(PATH);
    };
    let user = session.user;
    document(
        "Your profile",
        rsx! {
            ProfileView {
                name: user.name.clone().unwrap_or_default(),
                username: user.username.clone().unwrap_or_default(),
                image: user.image.clone().unwrap_or_default(),
                email: user.email.clone().unwrap_or_default(),
                email_verified: user.email_verified,
                flash: Flash::from_query(q.ok.as_deref(), q.error.as_deref()),
            }
        },
    )
}

/// `POST /account/profile` — name, username and avatar.
pub async fn save<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<ProfileForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    // An empty box means "clear this", which the engine spells
    // `Some("")`. Absent would mean "leave it alone", and there is no
    // way for a form to say that — every field is always submitted.
    let updated = state
        .auth
        .update_profile(UpdateProfile {
            session_token: token.clone(),
            name: Some(form.name.trim().to_owned()),
            image: Some(form.image.trim().to_owned()),
        })
        .await;
    if let Err(error) = updated {
        return flash_to(PATH, &Flash::Error(message(&error)));
    }

    // Username is a separate command because it is the only field here
    // that can collide with somebody else's.
    let username = form.username.trim();
    if !username.is_empty()
        && let Err(error) = state
            .auth
            .update_username(UpdateUsername {
                session_token: token,
                username: username.to_owned(),
                display_username: Some(form.username.trim().to_owned()),
            })
            .await
    {
        return flash_to(PATH, &Flash::Error(message(&error)));
    }
    flash_to(PATH, &Flash::Ok("Profile saved.".into()))
}

/// `POST /account/email`
pub async fn change_email<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<EmailForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    match state
        .auth
        .change_email(ChangeEmail {
            session_token: token,
            new_email: form.new_email.trim().to_owned(),
        })
        .await
    {
        // Deliberately not "your address is now X": depending on
        // configuration the change waits on the new address being
        // verified, and claiming it took effect when it has not is how
        // somebody locks themselves out of their own account.
        Ok(_) => flash_to(
            PATH,
            &Flash::Ok("Check the new address for a confirmation link.".into()),
        ),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

/// `POST /account/password`
pub async fn change_password<S>(
    State(state): State<UiState<S>>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = token_of(&headers, &state.cookie) else {
        return sign_in_first(PATH);
    };
    // Checked here rather than by the engine: a mistyped confirmation
    // is a typo in this form, not a fact about the account, and the
    // engine has no second field to compare against.
    if form.new_password != form.confirm_password {
        return flash_to(
            PATH,
            &Flash::Error("The new passwords do not match.".into()),
        );
    }
    match state
        .auth
        .change_password(ChangePassword {
            session_token: token,
            current_password: form.current_password,
            new_password: form.new_password,
        })
        .await
    {
        Ok(()) => flash_to(PATH, &Flash::Ok("Password changed.".into())),
        Err(error) => flash_to(PATH, &Flash::Error(message(&error))),
    }
}

async fn current<S>(
    state: &UiState<S>,
    headers: &HeaderMap,
) -> Option<architect_auth::proto::AuthSessionBundle>
where
    S: AuthStorage,
{
    let token = token_of(headers, &state.cookie)?;
    state
        .auth
        .current_session(CurrentSession { token })
        .await
        .ok()
}

/// The public wording for a flow error.
///
/// Through the same taxonomy the JSON API answers with, so a person and
/// a program are told the same thing, and neither is told anything the
/// taxonomy considers internal.
pub(crate) fn message(error: &architect_auth::proto::AuthFlowError) -> String {
    architect_auth::transport::map_auth_error(error)
        .message
        .to_owned()
}

#[component]
fn ProfileView(
    name: String,
    username: String,
    image: String,
    email: String,
    email_verified: bool,
    flash: Option<Flash>,
) -> Element {
    rsx! {
        h1 { "Your profile" }
        p { class: "sub", "Signed in as {email}" }
        FlashLine { flash }

        h2 { "Details" }
        form { method: "post", action: "{PATH}", class: "stack",
            label { r#for: "name", "Display name" }
            input { id: "name", name: "name", value: "{name}", autocomplete: "name" }

            label { r#for: "username", "Username" }
            input { id: "username", name: "username", value: "{username}", autocomplete: "username" }
            p { class: "hint", "Lowercase letters, digits and dashes. Others can find you by it." }

            label { r#for: "image", "Avatar URL" }
            input { id: "image", name: "image", value: "{image}", r#type: "url", placeholder: "https://…" }

            button { r#type: "submit", "Save profile" }
        }

        h2 { "Email address" }
        p { class: "hint",
            if email_verified {
                "{email} — verified."
            } else {
                "{email} — not verified yet."
            }
        }
        form { method: "post", action: "/account/email", class: "stack",
            label { r#for: "new_email", "New address" }
            input { id: "new_email", name: "new_email", r#type: "email", required: true, autocomplete: "email" }
            button { r#type: "submit", "Change address" }
        }

        h2 { "Password" }
        form { method: "post", action: "/account/password", class: "stack",
            label { r#for: "current_password", "Current password" }
            input { id: "current_password", name: "current_password", r#type: "password", required: true, autocomplete: "current-password" }

            label { r#for: "new_password", "New password" }
            input { id: "new_password", name: "new_password", r#type: "password", required: true, autocomplete: "new-password" }

            label { r#for: "confirm_password", "Confirm new password" }
            input { id: "confirm_password", name: "confirm_password", r#type: "password", required: true, autocomplete: "new-password" }

            button { r#type: "submit", "Change password" }
        }

        p { class: "alt",
            a { href: "/account", "Linked accounts" }
            " · "
            a { href: "/account/sessions", "Active sessions" }
            " · "
            a { href: "/orgs", "Organizations" }
        }
    }
}

#[component]
pub(crate) fn FlashLine(flash: Option<Flash>) -> Element {
    match flash {
        Some(Flash::Ok(message)) => rsx! { p { class: "ok", role: "status", "{message}" } },
        Some(Flash::Error(message)) => rsx! { p { class: "error", role: "alert", "{message}" } },
        None => rsx! {},
    }
}
