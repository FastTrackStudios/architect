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
            Shell {
                h1 { "Your account" }
                p { class: "sub", "Signed in as {email}" }

                match flash {
                    Some(Flash::Ok(message)) => rsx! { p { class: "ok", role: "status", "{message}" } },
                    Some(Flash::Error(message)) => rsx! { p { class: "error", role: "alert", "{message}" } },
                    None => rsx! {},
                }

                h2 { "Linked accounts" }
                p { class: "hint",
                    "Sign in with a linked account, and let the apps act as it. Task pushes and proposes the wiki edits you accept under your linked GitHub name."
                }
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
                                            span { class: "provider-name",
                                                ProviderMark { provider }
                                                span {
                                                    strong { "{name}" }
                                                    span { class: "handle", "Linked as {handle}" }
                                                }
                                            }
                                            form { method: "post", action: "/account/unlink", class: "inline",
                                                input { r#type: "hidden", name: "provider", value: "{id}" }
                                                button { r#type: "submit", class: "link", "Unlink" }
                                            }
                                        }
                                    }
                                }
                                None => rsx! {
                                    li { class: "provider",
                                        span { class: "provider-name",
                                            ProviderMark { provider }
                                            span {
                                                strong { "{name}" }
                                                span { class: "handle", "Not linked" }
                                            }
                                        }
                                        a { class: "button small", href: "/auth/social/{id}/start?mode=link&return_to=%2Faccount",
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
            Shell {
                h1 { "{title}" }
                p { class: "sub", "{message}" }
                p { class: "alt",
                    a { href: "/login?return_to={return_to}", "Go to sign in" }
                }
            }
        }
    }
}

/// The apps one account opens, each with the colour the studio gives it
/// on fasttrackstudio.app. The brand panel draws them as a short
/// spectrum — the site's own motif — with the names beneath.
const APPS: [(&str, &str); 5] = [
    ("Task", "#ededf1"),
    ("Keyflow", "#a78bfa"),
    ("Signal", "#2fd673"),
    ("Session", "#2e9bff"),
    ("Ignition", "#ff8a2b"),
];

/// The page frame every hosted screen sits in: the brand panel that says
/// what this account is for, and the working panel beside it. On a
/// narrow screen the brand panel folds into a short header so the form
/// is the first thing in reach.
#[component]
fn Shell(children: Element) -> Element {
    rsx! {
        div { class: "console",
            aside { class: "brand",
                a { class: "wordmark", href: "https://fasttrackstudio.app", "FastTrackStudio" }
                div { class: "pitch",
                    p { class: "tagline", "One account." }
                    p { class: "tagline dim", "Every app in the studio." }
                }
                ul { class: "apps", aria_label: "Apps this account signs in to",
                    for (name, color) in APPS {
                        li { style: "--app: {color}",
                            i { class: "bar" }
                            span { "{name}" }
                        }
                    }
                }
            }
            main { class: "panel", {children} }
        }
    }
}

/// A provider's own mark, inline so the page stays one request. GitHub's
/// takes the button's text colour; Google's keeps its four colours, as
/// its brand rules ask.
#[component]
fn ProviderMark(provider: Provider) -> Element {
    match provider {
        Provider::GitHub => rsx! {
            svg { class: "mark", view_box: "0 0 24 24", width: "20", height: "20", fill: "currentColor", "aria-hidden": "true",
                path { d: "M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.1.79-.25.79-.56v-2c-3.2.7-3.87-1.36-3.87-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.19 1.76 1.19 1.03 1.76 2.69 1.25 3.35.96.1-.75.4-1.25.73-1.54-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.29 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.8 1.19 1.83 1.19 3.09 0 4.42-2.69 5.39-5.26 5.68.41.36.78 1.05.78 2.12v3.14c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z" }
            }
        },
        Provider::Google => rsx! {
            svg { class: "mark", view_box: "0 0 24 24", width: "20", height: "20", "aria-hidden": "true",
                path { fill: "#4285F4", d: "M23.5 12.27c0-.79-.07-1.54-.2-2.27H12v4.3h6.46a5.53 5.53 0 0 1-2.4 3.63v3h3.87c2.27-2.09 3.57-5.17 3.57-8.66z" }
                path { fill: "#34A853", d: "M12 24c3.24 0 5.96-1.07 7.94-2.91l-3.87-3c-1.07.72-2.45 1.15-4.07 1.15-3.13 0-5.78-2.11-6.73-4.96H1.27v3.09A12 12 0 0 0 12 24z" }
                path { fill: "#FBBC05", d: "M5.27 14.28A7.2 7.2 0 0 1 4.9 12c0-.79.14-1.56.37-2.28V6.63H1.27A12 12 0 0 0 0 12c0 1.94.46 3.77 1.27 5.37l4-3.09z" }
                path { fill: "#EA4335", d: "M12 4.75c1.76 0 3.34.61 4.59 1.8l3.44-3.44C17.95 1.19 15.23 0 12 0A12 12 0 0 0 1.27 6.63l4 3.09C6.22 6.87 8.87 4.75 12 4.75z" }
            }
        },
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
            Shell {
                h1 { "{screen.title()}" }
                p { class: "sub",
                    match screen {
                        Screen::SignIn => "Welcome back.",
                        Screen::SignUp => "Free, and it works in every FastTrackStudio app.",
                        Screen::ForgotPassword => "Enter your email and we'll send a link to choose a new password.",
                        Screen::ResetPassword => "Pick something at least eight characters long.",
                    }
                }

                if let Some(message) = error {
                    p { class: "error", role: "alert", "{message}" }
                }

                // GitHub and Google first, on the two screens that sign
                // someone in, for the providers this deployment has.
                if matches!(screen, Screen::SignIn | Screen::SignUp) && !providers.is_empty() {
                    div { class: "social",
                        for provider in providers.iter().copied() {
                            a {
                                class: "button provider-button",
                                href: "/auth/social/{provider.id()}/start?mode=sign-in&return_to={encode_query_value(&return_to)}",
                                ProviderMark { provider }
                                span { "Continue with {provider.display_name()}" }
                            }
                        }
                    }
                    p { class: "or", span { "or with email" } }
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

                p { class: "alt",
                    // Encoded, not raw: see `encode_query_value`. The
                    // hidden form field below needs no such treatment —
                    // a POST body carries the value as one field.
                    {
                        let return_to = encode_query_value(&return_to);
                        match screen {
                            Screen::SignIn => rsx! {
                                span {
                                    "No account yet? "
                                    a { href: "/sign-up?return_to={return_to}", "Create one" }
                                }
                                a { class: "quiet", href: "/forgot-password?return_to={return_to}", "Forgot password?" }
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
@import url("https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@87.5..112.5,400..700&family=JetBrains+Mono:wght@400;700&display=swap");
:root {
  color-scheme: dark;
  /* fasttrackstudio.app's own tokens */
  --void: #08080a;
  --bg: #0a0a0c;
  --deck: #131318;
  --surface: #16161c;
  --raised: #1d1d25;
  --line: #26262f;
  --line-strong: #353541;
  --fg: #ededf1;
  --muted: #9c9ca8;
  --subtle: #63636f;
  --error: #ff8a7a;
  --ok: #2fd673;
  --sans: "Archivo", system-ui, -apple-system, "Segoe UI", sans-serif;
  --mono: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
}
* { box-sizing: border-box; }
html { background: var(--bg); }
body {
  margin: 0;
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 1.25rem;
  color: var(--fg);
  font: 15px/1.5 var(--sans);
  font-variation-settings: "wdth" 100;
  -webkit-font-smoothing: antialiased;
}
.console {
  width: 100%;
  max-width: 56rem;
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
  background: var(--deck);
  border: 1px solid var(--line);
  border-radius: 14px;
  overflow: hidden;
}
/* ── Brand panel ─────────────────────────────────────── */
.brand {
  display: flex;
  flex-direction: column;
  gap: 2.5rem;
  padding: 2.25rem 2rem;
  background: var(--void);
  border-right: 1px solid var(--line);
}
.wordmark {
  align-self: flex-start;
  margin: 0;
  color: var(--fg);
  text-decoration: none;
  font-weight: 700;
  font-size: .95rem;
  letter-spacing: .02em;
  text-transform: uppercase;
  font-variation-settings: "wdth" 95;
}
.pitch { margin-top: auto; }
.tagline {
  margin: 0;
  font-size: clamp(1.75rem, 3.2vw, 2.25rem);
  line-height: 1.05;
  font-weight: 600;
  letter-spacing: -.02em;
  font-variation-settings: "wdth" 92;
}
.tagline.dim { color: var(--muted); }
/* the site's spectrum motif: one bar per app, in the app's colour */
.apps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: .6rem;
}
.apps li {
  display: grid;
  gap: .55rem;
  font: 700 .6rem/1 var(--mono);
  letter-spacing: .18em;
  text-transform: uppercase;
  color: var(--subtle);
}
.apps .bar {
  display: block;
  height: 3px;
  border-radius: 2px;
  background: var(--app);
  opacity: .9;
}
/* ── Working panel ───────────────────────────────────── */
.panel {
  padding: 2.25rem 2.25rem 2rem;
  background: var(--deck);
}
h1 {
  margin: 0 0 .3rem;
  font-size: 1.6rem;
  line-height: 1.15;
  font-weight: 600;
  letter-spacing: -.015em;
  font-variation-settings: "wdth" 95;
}
h2 {
  margin: 1.75rem 0 .35rem;
  font-size: 1rem;
  font-weight: 600;
}
.sub { margin: 0 0 1.5rem; color: var(--muted); }
.hint { margin: 0 0 1rem; color: var(--muted); font-size: .9rem; }
label {
  display: block;
  margin: 0 0 .35rem;
  color: var(--muted);
  font-size: .85rem;
  font-weight: 500;
}
input {
  width: 100%;
  margin: 0 0 .9rem;
  padding: .7rem .8rem;
  font: inherit;
  color: var(--fg);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  border-radius: 8px;
}
input:hover { border-color: var(--subtle); }
input:focus-visible { outline: 2px solid var(--fg); outline-offset: 1px; border-color: var(--fg); }
button {
  width: 100%;
  margin-top: .25rem;
  padding: .75rem;
  font: inherit;
  font-weight: 600;
  color: var(--void);
  background: var(--fg);
  border: 0;
  border-radius: 8px;
  cursor: pointer;
}
button:hover { background: #fff; }
button:focus-visible { outline: 2px solid var(--fg); outline-offset: 2px; }
/* provider buttons: the mark, then the words, centred as one unit */
.social { display: grid; gap: .6rem; }
a.button {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: .65rem;
  padding: .72rem .9rem;
  text-decoration: none;
  font-weight: 600;
  color: var(--fg);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  border-radius: 8px;
}
a.button:hover { background: var(--raised); border-color: var(--subtle); }
a.button:focus-visible { outline: 2px solid var(--fg); outline-offset: 2px; }
a.button.small { display: inline-flex; padding: .45rem .8rem; font-size: .875rem; }
.mark { flex: none; }
.or {
  display: flex;
  align-items: center;
  gap: .9rem;
  margin: 1.25rem 0 1.1rem;
  color: var(--subtle);
  font-size: .8rem;
}
.or::before, .or::after { content: ""; flex: 1; height: 1px; background: var(--line); }
.error, .ok {
  margin: 0 0 1rem;
  padding: .65rem .8rem;
  font-size: .9rem;
  border-radius: 8px;
  border: 1px solid;
}
.error { color: var(--error); border-color: color-mix(in srgb, var(--error) 45%, transparent); }
.ok { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 45%, transparent); }
.alt { display: flex; justify-content: space-between; gap: 1rem; flex-wrap: wrap; margin: 1.5rem 0 0; color: var(--muted); font-size: .9rem; }
a.quiet { color: var(--muted); }
a.quiet:hover { color: var(--fg); }
a { color: var(--fg); text-underline-offset: .15em; }
a:hover { color: #fff; }
/* account: linked providers */
.providers { list-style: none; margin: 0; padding: 0; display: grid; }
.provider {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: .9rem 0;
  border-top: 1px solid var(--line);
}
.provider:last-child { border-bottom: 1px solid var(--line); }
.provider-name { display: flex; align-items: center; gap: .8rem; }
.provider-name strong { display: block; font-weight: 600; }
.handle { display: block; color: var(--muted); font-size: .85rem; }
form.inline { display: inline; margin: 0; }
button.link {
  width: auto;
  margin: 0;
  padding: 0;
  font-weight: 500;
  color: var(--muted);
  background: none;
  border: 0;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: .15em;
}
button.link:hover { color: var(--fg); background: none; }
@media (max-width: 52rem) {
  body { padding: 0; align-items: start; }
  .console { max-width: none; min-height: 100vh; grid-template-columns: 1fr; border: 0; border-radius: 0; }
  .brand { gap: 1.25rem; padding: 1.25rem 1.5rem; border-right: 0; border-bottom: 1px solid var(--line); }
  .pitch { display: none; }
  .apps { gap: .4rem; }
  .panel { padding: 1.75rem 1.5rem 2rem; }
}
@media (prefers-reduced-motion: no-preference) {
  a.button, button, input { transition: background-color .15s ease, border-color .15s ease; }
}
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
