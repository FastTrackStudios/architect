//! The pages a person actually sees.
//!
//! Everything else in this crate is an API: JSON in, JSON out, for a
//! program that already holds a credential. But `/oauth2/authorize`
//! needs a *browser* to be signed in before it can issue a code, and
//! until this module existed there was nowhere for that to happen — the
//! handler simply answered `401` and the redirect flow dead-ended. Its
//! own doc comment said "a front-end owns that"; this is that front-end.
//!
//! # Server-rendered, and no script at all
//!
//! Rendered with `dioxus-ssr` into plain HTML, with ordinary `<form>`
//! posts. That is a deliberate choice for *these* pages: a sign-in
//! screen is the one page that must work before anything else does, and
//! making someone download and boot a WASM bundle before they can type a
//! password buys nothing and fails badly — a slow network, a blocked
//! script, or a panic in the bundle all become "I cannot log in". A form
//! post has none of those failure modes.
//!
//! # Why the form posts here and not to the JSON endpoints
//!
//! `/auth/sign-in/email` takes JSON and answers JSON, which a browser
//! form cannot send and a person cannot read. These routes take
//! `application/x-www-form-urlencoded`, call exactly the same engine
//! methods, and answer with a redirect — so the two surfaces cannot
//! drift, because there is only one implementation underneath.

use architect_auth::{
    AuthStorage, CompletePasswordReset, CreateEmailPasswordUser, CurrentSession,
    RequestEmailVerification, RequestPasswordReset, VerifyEmail,
    transport::{AuthCookieConfig, axum::session_token_from_headers},
};
use auth_proto::{AuthSessionBundle, SignInEmailPassword};
use axum::{
    Router,
    extract::{Form, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use dioxus::prelude::*;

use crate::http::{HttpState, LinkedAccountView, UnlinkError, client_ip, user_agent};
use crate::social::Provider;

/// Where to send a browser that has just signed in, when the request
/// carried no `return_to` of its own.
const DEFAULT_AFTER_SIGN_IN: &str = "/";

/// Mount the sign-in and sign-up pages.
///
/// Merged next to [`crate::http::router`] rather than folded into it:
/// an embedder that already has its own login screen wants the API
/// without these, and keeping them separable is the difference between
/// "use our pages" and "use our server".
pub fn router<S>(state: HttpState<S>) -> Router
where
    S: AuthStorage + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/login", get(login_page::<S>).post(login_submit::<S>))
        .route("/sign-up", get(sign_up_page::<S>).post(sign_up_submit::<S>))
        // Password reset. These did not exist at all before there was a
        // mailer: the engine could mint a reset token and the server had
        // nowhere to send it, so a forgotten password meant an operator
        // editing the database by hand.
        .route(
            "/forgot-password",
            get(forgot_password_page).post(forgot_password_submit::<S>),
        )
        .route(
            "/reset-password",
            get(reset_password_page).post(reset_password_submit::<S>),
        )
        .route("/verify-email", get(verify_email_page::<S>))
        // The signed-in person's own page: which providers are linked,
        // and the buttons to link or unlink them.
        .route("/account", get(account_page::<S>))
        .route("/account/unlink", axum::routing::post(account_unlink::<S>))
        .with_state(state)
}

// ── Account page ─────────────────────────────────────────────────────

#[derive(Debug, Default, serde::Deserialize)]
pub struct AccountQuery {
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub linked: Option<String>,
    #[serde(default)]
    pub unlinked: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UnlinkForm {
    pub provider: String,
}

/// `GET /account` — sends a browser with no session to sign in first,
/// with this page as the place to come back to.
async fn account_page<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Query(q): Query<AccountQuery>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = session_token_from_headers(&headers, &state.cookie) else {
        return Redirect::to("/login?return_to=%2Faccount").into_response();
    };
    let bundle = match state.auth.current_session(CurrentSession { token }).await {
        Ok(bundle) => bundle,
        Err(_) => return Redirect::to("/login?return_to=%2Faccount").into_response(),
    };
    let accounts = crate::http::linked_accounts(&state, bundle.user.id)
        .await
        .unwrap_or_default();
    let flash = account_flash(&q);
    let body = dioxus_ssr::render_element(rsx! {
        AccountPage {
            email: bundle.user.email.clone().unwrap_or_default(),
            providers: state.social.enabled_providers(),
            accounts,
            flash,
        }
    });
    Html(format!("<!doctype html>\n<html lang=\"en\">{body}</html>")).into_response()
}

/// `POST /account/unlink` — the form twin of
/// `POST /auth/accounts/{provider}/unlink`, answering with a redirect
/// back to the page instead of JSON.
async fn account_unlink<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Form(form): Form<UnlinkForm>,
) -> Response
where
    S: AuthStorage,
{
    let Some(token) = session_token_from_headers(&headers, &state.cookie) else {
        return Redirect::to("/login?return_to=%2Faccount").into_response();
    };
    let Some(provider) = Provider::parse(&form.provider) else {
        return Redirect::to("/account?error=unknown_provider").into_response();
    };
    let location = match crate::http::unlink(&state, token, provider).await {
        Ok(()) => format!("/account?unlinked={}", provider.id()),
        Err(UnlinkError::NotLinked) => "/account?error=not_linked".to_owned(),
        Err(UnlinkError::LastCredential) => "/account?error=last_credential".to_owned(),
        Err(UnlinkError::Auth(_)) => "/account?error=unlink".to_owned(),
    };
    (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
}

/// A message for the page, from the `?error=`, `?linked=` and
/// `?unlinked=` codes the callback and unlink routes redirect with.
fn account_flash(q: &AccountQuery) -> Option<Flash> {
    if let Some(provider) = q.linked.as_deref().and_then(Provider::parse) {
        return Some(Flash::Ok(format!("{} linked.", provider.display_name())));
    }
    if let Some(provider) = q.unlinked.as_deref().and_then(Provider::parse) {
        return Some(Flash::Ok(format!("{} unlinked.", provider.display_name())));
    }
    let error = q.error.as_deref()?;
    Some(Flash::Error(describe_social_error(error).to_owned()))
}

/// The `?error=` codes in words. Shared with the login page, which gets
/// the sign-in ones.
fn describe_social_error(code: &str) -> &'static str {
    match code {
        "denied" => "The provider did not grant access.",
        "state" => "That sign-in link expired or was already used. Try again.",
        "exchange" | "profile" => "The provider could not be reached. Try again in a moment.",
        "email_in_use" => {
            "An account with that email already exists. Sign in with your password, then link the provider from your account page."
        }
        "no_email" => {
            "The provider did not share an email address, which is needed to create an account."
        }
        "already_linked" => "That account is already linked to a different user.",
        "session" => "Your session ended before the link finished. Sign in and try again.",
        "last_credential" => {
            "That is your only way to sign in. Set a password or link another provider before unlinking it."
        }
        "not_linked" => "That provider is not linked.",
        "unknown_provider" => "That provider is not available.",
        _ => "Something went wrong. Try again.",
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Flash {
    Ok(String),
    Error(String),
}

#[component]
fn AccountPage(
    email: String,
    providers: Vec<Provider>,
    accounts: Vec<LinkedAccountView>,
    flash: Option<Flash>,
) -> Element {
    rsx! {
        head {
            meta { charset: "utf-8" }
            meta { name: "viewport", content: "width=device-width, initial-scale=1" }
            title { "Your account · FastTrackStudio" }
            style { {STYLE} }
        }
        body {
            main { class: "card",
                h1 { "Your account" }
                p { class: "sub", "Signed in as {email}" }

                match flash {
                    Some(Flash::Ok(message)) => rsx! { p { class: "ok", role: "status", "{message}" } },
                    Some(Flash::Error(message)) => rsx! { p { class: "error", role: "alert", "{message}" } },
                    None => rsx! {},
                }

                h2 { "Linked accounts" }
                if providers.is_empty() {
                    p { class: "sub", "No providers are configured on this server." }
                }
                ul { class: "providers",
                    for provider in providers.iter().copied() {
                        {
                            let linked = accounts.iter().find(|a| a.provider_id == provider.id());
                            let name = provider.display_name();
                            let id = provider.id();
                            match linked {
                                Some(account) => {
                                    let handle = account.login.clone().unwrap_or_else(|| account.account_id.clone());
                                    rsx! {
                                        li { class: "provider",
                                            span { "Linked as " strong { "{handle}" } " · {name}" }
                                            form { method: "post", action: "/account/unlink", class: "inline",
                                                input { r#type: "hidden", name: "provider", value: "{id}" }
                                                button { r#type: "submit", class: "link", "Unlink" }
                                            }
                                        }
                                    }
                                }
                                None => rsx! {
                                    li { class: "provider",
                                        span { "{name}" }
                                        a { class: "button", href: "/auth/social/{id}/start?mode=link&return_to=%2Faccount",
                                            "Link {name}"
                                        }
                                    }
                                },
                            }
                        }
                    }
                }

                p { class: "alt",
                    form { method: "post", action: "/auth/sign-out", class: "inline",
                        button { r#type: "submit", class: "link", "Sign out" }
                    }
                }
            }
        }
    }
}

// ── Password reset ───────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct ForgotForm {
    pub email: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ResetForm {
    pub email: String,
    pub token: String,
    pub password: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct ResetQuery {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct VerifyQuery {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

async fn forgot_password_page(Query(q): Query<PageQuery>) -> Html<String> {
    Html(render(
        Screen::ForgotPassword,
        &safe_return_to(q.return_to.as_deref()),
        None,
    ))
}

async fn forgot_password_submit<S>(
    State(state): State<HttpState<S>>,
    Form(form): Form<ForgotForm>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_return_to(form.return_to.as_deref());

    if let Ok(token) = state
        .auth
        .request_password_reset(RequestPasswordReset {
            email: form.email.clone(),
        })
        .await
    {
        state
            .mail
            .send_password_reset(&form.email, &token.token)
            .await;
    }

    // The SAME answer whether or not the address is known. Anything else
    // is an account-enumeration oracle: "no such account" tells a stranger
    // exactly which of your users exist.
    notice(
        "Check your email",
        "If that address has an account, a reset link is on its way.          The link is valid for a short time.",
        &return_to,
    )
}

async fn reset_password_page(Query(q): Query<ResetQuery>) -> Html<String> {
    Html(render_with_reset(
        Screen::ResetPassword,
        &safe_return_to(q.return_to.as_deref()),
        None,
        q.email.as_deref().unwrap_or(""),
        q.token.as_deref().unwrap_or(""),
    ))
}

async fn reset_password_submit<S>(
    State(state): State<HttpState<S>>,
    Form(form): Form<ResetForm>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_return_to(form.return_to.as_deref());

    if form.password.chars().count() < 8 {
        return Html(render_with_reset(
            Screen::ResetPassword,
            &return_to,
            Some("Password must be at least 8 characters."),
            &form.email,
            &form.token,
        ))
        .into_response();
    }

    match state
        .auth
        .complete_password_reset(CompletePasswordReset {
            email: form.email.clone(),
            token: form.token.clone(),
            new_password: form.password,
        })
        .await
    {
        Ok(_) => notice(
            "Password changed",
            "You can sign in with your new password.",
            &return_to,
        ),
        // A used or expired link is the common case, not an attack, so it
        // says so plainly instead of failing blank.
        Err(_) => Html(render_with_reset(
            Screen::ResetPassword,
            &return_to,
            Some("That reset link is no longer valid. Ask for a new one."),
            &form.email,
            &form.token,
        ))
        .into_response(),
    }
}

async fn verify_email_page<S>(
    State(state): State<HttpState<S>>,
    Query(q): Query<VerifyQuery>,
) -> Response
where
    S: AuthStorage,
{
    let parsed = q
        .user_id
        .as_deref()
        .and_then(|id| uuid::Uuid::parse_str(id).ok());

    let ok = match (parsed, q.token.as_deref()) {
        (Some(user_id), Some(token)) => state
            .auth
            .verify_email(VerifyEmail {
                user_id,
                token: token.to_owned(),
            })
            .await
            .is_ok(),
        _ => false,
    };

    if ok {
        notice(
            "Email confirmed",
            "Your account is ready. You can sign in.",
            "/",
        )
    } else {
        notice(
            "That link did not work",
            "It may have already been used, or expired. Sign in and ask for another.",
            "/",
        )
    }
}

/// A plain outcome page in the same shell as the forms.
fn notice(title: &str, message: &str, return_to: &str) -> Response {
    let body = dioxus_ssr::render_element(rsx! {
        Notice {
            title: title.to_owned(),
            message: message.to_owned(),
            return_to: return_to.to_owned(),
        }
    });
    Html(format!("<!doctype html>\n<html lang=\"en\">{body}</html>")).into_response()
}

// ── Redirect safety ──────────────────────────────────────────────────

/// Reduce a caller-supplied `return_to` to something safe to `Location:`.
///
/// An unchecked `return_to` is an open redirect, and on an *identity*
/// server that is worth more than usual: the phishing page it forwards
/// to is reached through a link that genuinely begins at the real login
/// screen, having genuinely signed the person in.
///
/// Only same-origin absolute paths survive. `//evil.example` is rejected
/// along with every scheme-bearing URL — a leading `//` is a
/// protocol-relative URL, which browsers treat as cross-origin even
/// though it looks like a path.
fn safe_return_to(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or("").trim();
    let ok = candidate.starts_with('/')
        && !candidate.starts_with("//")
        && !candidate.contains(['\\', '\r', '\n']);
    if ok {
        candidate.to_owned()
    } else {
        DEFAULT_AFTER_SIGN_IN.to_owned()
    }
}

/// Percent-encode a value that is going INSIDE a query-string parameter.
///
/// `return_to` is itself a URL carrying its own query —
/// `/oauth2/authorize?client_id=forum&redirect_uri=…&state=…` — so
/// interpolating it raw into `?return_to={}` let the browser read its `&`
/// as separators for the OUTER URL. `return_to` truncated at the first
/// one, the rest arrived as siblings of `/sign-up`, and the redirect after
/// a successful sign-up landed on `/oauth2/authorize?client_id=forum` with
/// no `redirect_uri` at all — "Failed to deserialize query string:
/// missing field `redirect_uri`", with the account already created, so the person was left stranded
/// with a working account and no way back to what they were signing in to.
///
/// `/` is left alone: it is legal in a query value and keeps the link
/// readable.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub return_to: Option<String>,
    /// A short code from a failed social sign-in, shown in words.
    #[serde(default)]
    pub error: Option<String>,
}

// ── GET ──────────────────────────────────────────────────────────────

async fn login_page<S>(
    State(state): State<HttpState<S>>,
    Query(q): Query<PageQuery>,
) -> Html<String>
where
    S: AuthStorage,
{
    Html(render_page(
        Screen::SignIn,
        &safe_return_to(q.return_to.as_deref()),
        q.error.as_deref().map(describe_social_error),
        &state.social.enabled_providers(),
    ))
}

async fn sign_up_page<S>(
    State(state): State<HttpState<S>>,
    Query(q): Query<PageQuery>,
) -> Html<String>
where
    S: AuthStorage,
{
    Html(render_page(
        Screen::SignUp,
        &safe_return_to(q.return_to.as_deref()),
        q.error.as_deref().map(describe_social_error),
        &state.social.enabled_providers(),
    ))
}

// ── POST ─────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct SignInForm {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct SignUpForm {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub return_to: Option<String>,
}

async fn login_submit<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Form(form): Form<SignInForm>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_return_to(form.return_to.as_deref());
    let result = state
        .auth
        .sign_in_email_password(SignInEmailPassword {
            email: form.email,
            password: form.password,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await;

    match result {
        Ok(bundle) => signed_in(&state.cookie, &bundle, &return_to),
        // Deliberately not distinguishing "no such account" from "wrong
        // password": the difference is an account-enumeration oracle,
        // and a person who mistyped either one does the same thing next.
        Err(_) => rejected(
            Screen::SignIn,
            &return_to,
            "Email or password is incorrect.",
        ),
    }
}

async fn sign_up_submit<S>(
    State(state): State<HttpState<S>>,
    headers: HeaderMap,
    Form(form): Form<SignUpForm>,
) -> Response
where
    S: AuthStorage,
{
    let return_to = safe_return_to(form.return_to.as_deref());

    // Checked here rather than only in the browser, because `required`
    // and `minlength` on the input are a convenience for the person, not
    // a control — this endpoint is reachable without them.
    if form.password.chars().count() < 8 {
        return rejected(
            Screen::SignUp,
            &return_to,
            "Password must be at least 8 characters.",
        );
    }

    let result = state
        .auth
        .create_email_password_user(CreateEmailPasswordUser {
            email: form.email,
            password: form.password,
            name: form.name.filter(|n| !n.trim().is_empty()),
            username: None,
            image: None,
            metadata_json: None,
            ip_address: client_ip(&headers),
            user_agent: user_agent(&headers),
        })
        .await;

    match result {
        Ok(bundle) => {
            // Best effort, and deliberately after the account exists. A
            // provider having a bad minute must not turn a successful
            // sign-up into a failed one — the account is real either way,
            // and another link can always be asked for.
            if let Ok(token) = state
                .auth
                .request_email_verification(RequestEmailVerification {
                    user_id: bundle.user.id,
                })
                .await
                && let Some(email) = bundle.user.email.clone()
            {
                state
                    .mail
                    .send_email_verification(&email, bundle.user.id, &token.token)
                    .await;
            }
            signed_in(&state.cookie, &bundle, &return_to)
        }
        Err(e) => rejected(Screen::SignUp, &return_to, &describe_sign_up_error(&e)),
    }
}

/// A sign-up failure a person can act on.
///
/// Unlike sign-in, saying "that address is taken" is the *useful* answer
/// and leaks nothing a sign-up attempt would not reveal anyway.
fn describe_sign_up_error(error: &impl std::fmt::Display) -> String {
    let text = error.to_string();
    let lower = text.to_lowercase();
    if lower.contains("exist") || lower.contains("taken") || lower.contains("duplicate") {
        "An account with that email already exists — sign in instead.".to_owned()
    } else {
        text
    }
}

/// Set the session cookie and send the browser on.
///
/// 303, not 302: the browser must switch to GET for the redirect, or a
/// refresh on the destination re-submits the credentials.
fn signed_in(cookie: &AuthCookieConfig, bundle: &AuthSessionBundle, return_to: &str) -> Response {
    let set_cookie = cookie.session_cookie(bundle.token.clone());
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, set_cookie.to_string()),
            (header::LOCATION, return_to.to_owned()),
        ],
    )
        .into_response()
}

/// Re-render the screen with the reason it failed.
///
/// 200, not 4xx: this is a page a person reads, and some browsers and
/// intermediaries substitute their own body for an error status.
fn rejected(screen: Screen, return_to: &str, message: &str) -> Response {
    Html(render(screen, return_to, Some(message))).into_response()
}

// ── Rendering ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    SignIn,
    SignUp,
    /// "Send me a reset link" — email only.
    ForgotPassword,
    /// "Choose a new password" — password only, with the token carried
    /// in hidden fields.
    ResetPassword,
}

impl Screen {
    fn title(self) -> &'static str {
        match self {
            Screen::SignIn => "Sign in",
            Screen::SignUp => "Create your account",
            Screen::ForgotPassword => "Reset your password",
            Screen::ResetPassword => "Choose a new password",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Screen::SignIn => "/login",
            Screen::SignUp => "/sign-up",
            Screen::ForgotPassword => "/forgot-password",
            Screen::ResetPassword => "/reset-password",
        }
    }

    fn submit(self) -> &'static str {
        match self {
            Screen::SignIn => "Sign in",
            Screen::SignUp => "Create account",
            Screen::ForgotPassword => "Send reset link",
            Screen::ResetPassword => "Set password",
        }
    }

    /// Whether this screen asks for an email address.
    fn wants_email(self) -> bool {
        !matches!(self, Screen::ResetPassword)
    }

    /// Whether this screen asks for a password.
    fn wants_password(self) -> bool {
        !matches!(self, Screen::ForgotPassword)
    }
}

fn render(screen: Screen, return_to: &str, error: Option<&str>) -> String {
    render_full(screen, return_to, error, "", "", &[])
}

/// As [`render`], with the social buttons for the configured providers.
fn render_page(
    screen: Screen,
    return_to: &str,
    error: Option<&str>,
    providers: &[Provider],
) -> String {
    render_full(screen, return_to, error, "", "", providers)
}

/// As [`render`], but carrying the credentials a password reset needs.
/// Kept separate so the four existing call sites do not grow two empty
/// arguments they have no use for.
fn render_with_reset(
    screen: Screen,
    return_to: &str,
    error: Option<&str>,
    reset_email: &str,
    reset_token: &str,
) -> String {
    render_full(screen, return_to, error, reset_email, reset_token, &[])
}

fn render_full(
    screen: Screen,
    return_to: &str,
    error: Option<&str>,
    reset_email: &str,
    reset_token: &str,
    providers: &[Provider],
) -> String {
    let body = dioxus_ssr::render_element(rsx! {
        Page {
            screen,
            return_to: return_to.to_owned(),
            error: error.map(std::borrow::ToOwned::to_owned),
            reset_email: reset_email.to_owned(),
            reset_token: reset_token.to_owned(),
            providers: providers.to_vec(),
        }
    });
    format!("<!doctype html>\n<html lang=\"en\">{body}</html>")
}

/// The outcome pages — confirmed, link expired, check your email. Same
/// shell as the forms so the flow does not visibly change hands.
#[component]
fn Notice(title: String, message: String, return_to: String) -> Element {
    rsx! {
        head {
            meta { charset: "utf-8" }
            meta { name: "viewport", content: "width=device-width, initial-scale=1" }
            title { "{title}" }
            style { {STYLE} }
        }
        body {
            main { class: "card",
                h1 { "{title}" }
                p { class: "sub", "{message}" }
                p { class: "alt",
                    a { href: "/login?return_to={return_to}", "Go to sign in" }
                }
            }
        }
    }
}

#[component]
fn Page(
    screen: Screen,
    return_to: String,
    error: Option<String>,
    reset_email: String,
    reset_token: String,
    providers: Vec<Provider>,
) -> Element {
    rsx! {
        head {
            meta { charset: "utf-8" }
            meta {
                name: "viewport",
                content: "width=device-width, initial-scale=1",
            }
            title { "{screen.title()} · FastTrackStudio" }
            style { {STYLE} }
        }
        body {
            main { class: "card",
                h1 { "{screen.title()}" }
                p { class: "sub", "One account for every FastTrackStudio app." }

                if let Some(message) = error {
                    p { class: "error", role: "alert", "{message}" }
                }

                form { method: "post", action: screen.action(),
                    input { r#type: "hidden", name: "return_to", value: "{return_to}" }

                    if screen == Screen::SignUp {
                        label { r#for: "name", "Name" }
                        input {
                            id: "name",
                            name: "name",
                            r#type: "text",
                            autocomplete: "name",
                        }
                    }

                    if screen.wants_email() {
                        label { r#for: "email", "Email" }
                        input {
                            id: "email",
                            name: "email",
                            r#type: "email",
                            autocomplete: "email",
                            required: true,
                            autofocus: true,
                        }
                    }

                    // The reset token rides in the form rather than
                    // staying in the URL, so choosing a new password is a
                    // POST and the token never lands in a Referer header
                    // on the way out.
                    if screen == Screen::ResetPassword {
                        input { r#type: "hidden", name: "email", value: "{reset_email}" }
                        input { r#type: "hidden", name: "token", value: "{reset_token}" }
                    }

                    if screen.wants_password() {
                    label { r#for: "password", "Password" }
                    input {
                        id: "password",
                        name: "password",
                        r#type: "password",
                        // Tells a password manager to offer a new
                        // password on sign-up and the saved one on
                        // sign-in; the wrong value here is why managers
                        // sometimes refuse to fill.
                        autocomplete: if screen == Screen::SignUp { "new-password" } else { "current-password" },
                        required: true,
                        minlength: if matches!(screen, Screen::SignUp | Screen::ResetPassword) { "8" } else { "1" },
                    }
                    }

                    button { r#type: "submit", "{screen.submit()}" }
                }

                // "Continue with GitHub / Google" — only on the two
                // screens that sign someone in, and only for providers
                // this deployment has configured.
                if matches!(screen, Screen::SignIn | Screen::SignUp) && !providers.is_empty() {
                    div { class: "social",
                        p { class: "or", "or" }
                        for provider in providers.iter().copied() {
                            a {
                                class: "button secondary",
                                href: "/auth/social/{provider.id()}/start?mode=sign-in&return_to={encode_query_value(&return_to)}",
                                "Continue with {provider.display_name()}"
                            }
                        }
                    }
                }

                p { class: "alt",
                    // Encoded, not raw: see `encode_query_value`. The
                    // hidden form field below needs no such treatment —
                    // a POST body carries the value as one field.
                    {
                        let return_to = encode_query_value(&return_to);
                        match screen {
                            Screen::SignIn => rsx! {
                                "No account yet? "
                                a { href: "/sign-up?return_to={return_to}", "Create one" }
                                " · "
                                a { href: "/forgot-password?return_to={return_to}", "Forgot password?" }
                            },
                            Screen::SignUp => rsx! {
                                "Already have an account? "
                                a { href: "/login?return_to={return_to}", "Sign in" }
                            },
                            _ => rsx! {
                                a { href: "/login?return_to={return_to}", "Back to sign in" }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// Inlined rather than served as a file.
///
/// One request, no cache to bust, and nothing to 404 — the page cannot
/// arrive unstyled because the styles cannot arrive separately. It is
/// small enough that this costs less than the extra round trip would.
const STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #f6f7f9;
  --card: #ffffff;
  --fg: #14161a;
  --muted: #5c6370;
  --line: #d8dce3;
  --accent: #2f6fed;
  --error: #b3261e;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0f1115;
    --card: #171a21;
    --fg: #e8eaed;
    --muted: #9aa1ad;
    --line: #2a2f3a;
    --accent: #6c9bff;
    --error: #f2b8b5;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 1.5rem;
  background: var(--bg);
  color: var(--fg);
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
}
.card {
  width: 100%;
  max-width: 22rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: 12px;
  padding: 2rem;
}
h1 { margin: 0 0 .25rem; font-size: 1.4rem; }
.sub { margin: 0 0 1.5rem; color: var(--muted); font-size: .9rem; }
label { display: block; margin: 0 0 .35rem; font-size: .85rem; font-weight: 600; }
input {
  width: 100%;
  margin: 0 0 1rem;
  padding: .6rem .7rem;
  font: inherit;
  color: var(--fg);
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 8px;
}
input:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
button {
  width: 100%;
  padding: .65rem;
  font: inherit;
  font-weight: 600;
  color: #fff;
  background: var(--accent);
  border: 0;
  border-radius: 8px;
  cursor: pointer;
}
button:hover { filter: brightness(1.07); }
.error {
  margin: 0 0 1rem;
  padding: .6rem .7rem;
  font-size: .875rem;
  color: var(--error);
  border: 1px solid var(--error);
  border-radius: 8px;
}
.alt { margin: 1.25rem 0 0; font-size: .875rem; color: var(--muted); text-align: center; }
a { color: var(--accent); }
.ok {
  margin: 0 0 1rem;
  padding: .6rem .7rem;
  font-size: .875rem;
  color: var(--accent);
  border: 1px solid var(--accent);
  border-radius: 8px;
}
h2 { margin: 1.5rem 0 .5rem; font-size: 1rem; }
.social { margin-top: 1rem; display: grid; gap: .5rem; }
.or { margin: 0; text-align: center; color: var(--muted); font-size: .8rem; }
a.button {
  display: block;
  padding: .6rem;
  text-align: center;
  text-decoration: none;
  font-weight: 600;
  border-radius: 8px;
  border: 1px solid var(--line);
  color: var(--fg);
  background: var(--bg);
}
a.button:hover { border-color: var(--accent); }
.providers { list-style: none; margin: 0; padding: 0; display: grid; gap: .75rem; }
.provider {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  font-size: .9rem;
}
.provider a.button { display: inline-block; padding: .4rem .8rem; }
form.inline { display: inline; margin: 0; }
button.link {
  width: auto;
  padding: 0;
  font-weight: 500;
  color: var(--accent);
  background: none;
  border: 0;
  cursor: pointer;
  text-decoration: underline;
}
button.link:hover { filter: none; }
"#;

#[cfg(test)]
mod tests {
    use super::{Screen, render, safe_return_to};

    /// The open-redirect fence. Each of these, forwarded verbatim in a
    /// `Location:`, sends a freshly-signed-in person to someone else's
    /// site from a link that started at the real login screen.
    #[test]
    fn only_same_origin_paths_survive() {
        assert_eq!(
            safe_return_to(Some("/oauth2/authorize?x=1")),
            "/oauth2/authorize?x=1"
        );

        for hostile in [
            "//evil.example",
            "https://evil.example",
            "http://evil.example",
            "javascript:alert(1)",
            "/\\evil.example",
            "/ok\r\nLocation: https://evil.example",
        ] {
            assert_eq!(
                safe_return_to(Some(hostile)),
                "/",
                "{hostile} should not survive"
            );
        }
    }

    /// The bug this pins: a real sign-up at the forum's issuer created the
    /// account, then dumped the person on
    /// `/oauth2/authorize?client_id=forum` — everything from the first `&`
    /// onwards had been eaten, so the authorize endpoint refused it with
    /// "missing field `redirect_uri`". `return_to` carries a URL with its
    /// own query, and it was being interpolated raw into another one.
    #[test]
    fn a_return_to_carrying_its_own_query_survives_the_round_trip() {
        let return_to = "/oauth2/authorize?client_id=forum                         &redirect_uri=https%3A%2F%2Fforum.example%2Fcb                         &response_type=code&scope=openid+email&state=abc";

        let html = render(Screen::SignIn, return_to, None);
        let href = html
            .split("href=\"")
            .find(|part| part.starts_with("/sign-up"))
            .expect("a sign-up link")
            .split('"')
            .next()
            .unwrap();

        // Exactly one parameter on the OUTER url. A bare `&` here is the
        // whole bug: the browser would read it as a separator.
        let query = href.strip_prefix("/sign-up?").expect("a query");
        assert!(
            !query.contains('&') && !query.contains("&#38;"),
            "return_to leaked separators into the outer query: {href}"
        );

        // And it still means the same thing once decoded.
        let value = query.strip_prefix("return_to=").expect("return_to");
        assert_eq!(decode(value), return_to);
    }

    fn decode(value: &str) -> String {
        let bytes = value.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn a_missing_return_to_is_the_default() {
        assert_eq!(safe_return_to(None), "/");
        assert_eq!(safe_return_to(Some("   ")), "/");
    }

    /// The form has to carry `return_to` forward, or the resumed
    /// authorize request is lost the moment someone signs in.
    #[test]
    fn the_form_carries_the_destination() {
        let html = render(Screen::SignIn, "/oauth2/authorize?client_id=task", None);
        assert!(html.contains(r#"name="return_to""#));
        assert!(html.contains("/oauth2/authorize?client_id=task"));
        assert!(html.contains(r#"action="/login""#));
    }

    #[test]
    fn an_error_is_shown_to_the_person() {
        let html = render(Screen::SignIn, "/", Some("Email or password is incorrect."));
        assert!(html.contains("Email or password is incorrect."));
        assert!(html.contains(r#"role="alert""#));
    }

    /// Password managers key off these; the wrong one is why they
    /// sometimes refuse to fill or offer to save.
    #[test]
    fn sign_up_and_sign_in_ask_for_different_passwords() {
        assert!(render(Screen::SignUp, "/", None).contains("new-password"));
        assert!(render(Screen::SignIn, "/", None).contains("current-password"));
    }

    #[test]
    fn each_screen_offers_the_other() {
        assert!(render(Screen::SignIn, "/", None).contains("/sign-up"));
        assert!(render(Screen::SignUp, "/", None).contains("/login"));
    }
}
