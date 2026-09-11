//! A stand-in for GitHub, Google and TONE3000.
//!
//! # What it is for
//!
//! Social sign-in is the one part of an identity server that cannot be
//! demonstrated, developed against, or tested end to end without
//! credentials from three companies and a public callback URL. So the
//! buttons were simply absent from every local run, and the linking
//! code — the part that decides *who somebody is* — was exercised only
//! by unit tests with a fake client.
//!
//! This is the other half: a real HTTP server that speaks enough of the
//! authorization-code flow for the real client to complete it, and
//! answers with each provider's own profile shape.
//!
//! # What it is not
//!
//! Not an OAuth implementation. It does not verify the client secret,
//! rotate anything, expire anything, or check PKCE. It exists to be
//! *believed*, and it says so on every page it renders, because a mock
//! that looked production-shaped would eventually be pointed at by
//! something that mattered.
//!
//! The authorize page shows a fixed cast to pick from rather than a
//! login form. Picking is the whole interaction: the point is to see
//! what happens *after* the provider says yes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

/// One fake account, and everything the three providers might say about
/// it. Each provider renders the subset it actually returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub login: String,
    pub name: String,
    pub email: String,
    pub avatar: String,
}

impl Account {
    fn new(id: &str, login: &str, name: &str, email: &str, avatar: &str) -> Self {
        Self {
            id: id.to_owned(),
            login: login.to_owned(),
            name: name.to_owned(),
            email: email.to_owned(),
            avatar: avatar.to_owned(),
        }
    }
}

/// The cast, per provider.
///
/// Fixed and small on purpose: a browser test can name `octocat` and a
/// person demonstrating the flow can pick the same account twice and
/// get the same identity, which is what makes "link" distinguishable
/// from "sign in".
#[must_use]
pub fn accounts(provider: &str) -> Vec<Account> {
    match provider {
        "github" => vec![
            Account::new(
                "1001",
                "octocat",
                "Mona Lisa Octocat",
                "octocat@github.local",
                "https://avatars.githubusercontent.com/u/583231?v=4",
            ),
            Account::new(
                "1002",
                "hubot",
                "Hubot",
                "hubot@github.local",
                "https://avatars.githubusercontent.com/u/2?v=4",
            ),
        ],
        "google" => vec![
            Account::new(
                "google-oauth2|2001",
                "ada.lovelace",
                "Ada Lovelace",
                "ada@google.local",
                "https://lh3.googleusercontent.com/a/default-user",
            ),
            Account::new(
                "google-oauth2|2002",
                "grace.hopper",
                "Grace Hopper",
                "grace@google.local",
                "https://lh3.googleusercontent.com/a/default-user",
            ),
        ],
        "tone3000" => vec![
            Account::new(
                "3f7a1c90-0000-4000-8000-000000003001",
                "tone-tinkerer",
                "Tone Tinkerer",
                // TONE3000 returns no address, and the auth server
                // deliberately ignores one if it did — see
                // `fetch_profile`. Empty here so one table can describe
                // all three providers.
                "",
                "https://www.tone3000.com/avatar.png",
            ),
            Account::new(
                "3f7a1c90-0000-4000-8000-000000003002",
                "amp-hoarder",
                "Amp Hoarder",
                "",
                "https://www.tone3000.com/avatar.png",
            ),
        ],
        _ => Vec::new(),
    }
}

/// The account an id stands for — invented if it is not in the cast.
///
/// The cast is shared, so two browsers running the same test in
/// parallel would link the same provider account and the second would
/// correctly be refused as already linked. Rather than grow the cast
/// until collisions are unlikely, any handle at all is an account here:
/// a test that needs an identity nobody else has can simply name one.
#[must_use]
pub fn account(provider: &str, id: &str) -> Option<Account> {
    if accounts(provider).is_empty() {
        return None;
    }
    if let Some(known) = accounts(provider).into_iter().find(|a| a.id == id) {
        return Some(known);
    }
    let handle = id.trim();
    if handle.is_empty() {
        return None;
    }
    Some(Account::new(
        handle,
        handle,
        handle,
        // TONE3000 has no addresses, invented or otherwise.
        &if provider == "tone3000" {
            String::new()
        } else {
            format!("{handle}@{provider}.local")
        },
        "",
    ))
}

/// Codes handed out, and the account each stands for.
///
/// In memory and never cleaned up: this process is expected to live for
/// the length of a demo.
#[derive(Clone, Default)]
pub struct Issued(Arc<Mutex<HashMap<String, (String, String)>>>);

impl Issued {
    fn put(&self, code: String, provider: String, account_id: String) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(code, (provider, account_id));
        }
    }

    fn take(&self, code: &str) -> Option<(String, String)> {
        // Single-use, like the real thing — a replayed code is the most
        // likely bug in a client, and a mock that allowed it would hide
        // that bug until production.
        self.0.lock().ok().and_then(|mut map| map.remove(code))
    }
}

/// Every route this mock serves.
#[must_use]
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route(
            "/{provider}/authorize",
            get(authorize_page).post(authorize_submit),
        )
        .route("/{provider}/token", post(token))
        .route("/{provider}/user", get(user))
        .route("/{provider}/user/emails", get(emails))
        .with_state(Issued::default())
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct AuthorizeQuery {
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct PickForm {
    pub account_id: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct TokenForm {
    #[serde(default)]
    pub code: String,
}

async fn index() -> Html<String> {
    Html(page(
        "Mock OAuth",
        &format!(
            "<p>Standing in for GitHub, Google and TONE3000.</p>\
             <p class=\"warn\">Nothing here is checked. No secret is \
             verified, no token expires, and every account is invented. \
             Never point anything real at this.</p>\
             <p>Endpoints: {}</p>",
            ["github", "google", "tone3000"]
                .map(|p| format!("<code>/{p}/authorize</code>"))
                .join(", ")
        ),
    ))
}

/// `GET /{provider}/authorize` — pick who to be.
async fn authorize_page(Path(provider): Path<String>, Query(q): Query<AuthorizeQuery>) -> Response {
    let cast = accounts(&provider);
    if cast.is_empty() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Html(page(
                "Unknown provider",
                &format!("<p>No mock for <code>{}</code>.</p>", escape(&provider)),
            )),
        )
            .into_response();
    }
    let mut rows = String::new();
    for account in &cast {
        use std::fmt::Write as _;
        // `write!` into a String cannot fail; the result is discarded
        // rather than unwrapped so this stays panic-free.
        drop(write!(
            rows,
            "<form method=\"post\">\
               <input type=\"hidden\" name=\"account_id\" value=\"{id}\">\
               <input type=\"hidden\" name=\"redirect_uri\" value=\"{redirect}\">\
               <input type=\"hidden\" name=\"state\" value=\"{state}\">\
               <button type=\"submit\"><strong>{name}</strong><span>{login}</span></button>\
             </form>",
            id = escape(&account.id),
            redirect = escape(&q.redirect_uri),
            state = escape(&q.state),
            name = escape(&account.name),
            login = escape(&account.login),
        ));
    }

    Html(page(
        &format!("Continue with {provider}"),
        &format!(
            "<p class=\"warn\">This is a mock. Choose an account to \
             pretend to be.</p>\
             <p class=\"meta\">client_id <code>{client}</code><br>scope \
             <code>{scope}</code></p>\
             <div class=\"cast\">{rows}</div>\
             <form method=\"post\" class=\"invent\">\
               <input type=\"hidden\" name=\"redirect_uri\" value=\"{redirect}\">\
               <input type=\"hidden\" name=\"state\" value=\"{state}\">\
               <label for=\"account_id\">Or somebody new</label>\
               <div class=\"row\">\
                 <input id=\"account_id\" name=\"account_id\" placeholder=\"a handle\" \
                        autocomplete=\"off\" required>\
                 <button type=\"submit\">Continue</button>\
               </div>\
             </form>",
            client = escape(&q.client_id),
            scope = escape(if q.scope.is_empty() {
                "(none)"
            } else {
                &q.scope
            }),
            redirect = escape(&q.redirect_uri),
            state = escape(&q.state),
        ),
    ))
    .into_response()
}

/// `POST /{provider}/authorize` — hand back a code.
async fn authorize_submit(
    Path(provider): Path<String>,
    State(issued): State<Issued>,
    Form(form): Form<PickForm>,
) -> Response {
    let Some(picked) = account(&provider, &form.account_id) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Html(page(
                "Nobody picked",
                "<p>Choose an account or name a handle.</p>",
            )),
        )
            .into_response();
    };
    let code = uuid::Uuid::new_v4().simple().to_string();
    issued.put(code.clone(), provider, picked.id);
    let separator = if form.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    Redirect::to(&format!(
        "{}{separator}code={code}&state={}",
        form.redirect_uri,
        urlencode(&form.state)
    ))
    .into_response()
}

/// `POST /{provider}/token`
///
/// The access token *is* the account id, prefixed. A real provider's
/// token is opaque and needs a lookup; here the userinfo endpoint can
/// simply read it back, which keeps the mock stateless after this point
/// and makes a failure obvious in a log.
async fn token(
    Path(provider): Path<String>,
    State(issued): State<Issued>,
    Form(form): Form<TokenForm>,
) -> Response {
    let Some((issued_provider, account_id)) = issued.take(&form.code) else {
        return Json(json!({
            "error": "invalid_grant",
            "error_description": "unknown or already-used code",
        }))
        .into_response();
    };
    if issued_provider != provider {
        return Json(json!({
            "error": "invalid_grant",
            "error_description": "code was issued for another provider",
        }))
        .into_response();
    }
    Json(json!({
        "access_token": format!("mock.{provider}.{account_id}"),
        "refresh_token": format!("mock-refresh.{provider}.{account_id}"),
        "token_type": "bearer",
        "expires_in": 3600,
        "scope": "",
    }))
    .into_response()
}

/// The account a bearer token stands for.
fn account_from_token(headers: &axum::http::HeaderMap, provider: &str) -> Option<Account> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    let id = token.strip_prefix(&format!("mock.{provider}."))?;
    account(provider, id)
}

/// `GET /{provider}/user` — each provider's own profile shape.
async fn user(Path(provider): Path<String>, headers: axum::http::HeaderMap) -> Response {
    let Some(account) = account_from_token(&headers, &provider) else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "bad credentials" })),
        )
            .into_response();
    };
    // Three different shapes, because the auth server parses three
    // different shapes. A mock that returned one common envelope would
    // pass its own tests and fail against the real providers.
    match provider.as_str() {
        "github" => Json(json!({
            // GitHub's id is a NUMBER, and an invented handle has none —
            // so it gets a stable one derived from the handle itself,
            // which keeps the numeric branch of the parser exercised.
            "id": account.id.parse::<i64>().unwrap_or_else(|_| stable_number(&account.id)),
            "login": account.login,
            "name": account.name,
            // Null, so the flow falls through to /user/emails, which is
            // the path that actually matters for matching an identity.
            "email": serde_json::Value::Null,
            "avatar_url": account.avatar,
        }))
        .into_response(),
        "google" => Json(json!({
            "sub": account.id,
            "email": account.email,
            "email_verified": true,
            "name": account.name,
            "picture": account.avatar,
        }))
        .into_response(),
        "tone3000" => Json(json!({
            "id": account.id,
            "username": account.login,
            "avatar_url": account.avatar,
        }))
        .into_response(),
        _ => (axum::http::StatusCode::NOT_FOUND, Json(json!({}))).into_response(),
    }
}

/// `GET /github/user/emails`
async fn emails(Path(provider): Path<String>, headers: axum::http::HeaderMap) -> Response {
    let Some(account) = account_from_token(&headers, &provider) else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "bad credentials" })),
        )
            .into_response();
    };
    Json(json!([
        { "email": account.email, "primary": true, "verified": true },
        { "email": format!("alt+{}", account.email), "primary": false, "verified": false },
    ]))
    .into_response()
}

/// A small positive number that is always the same for the same handle.
fn stable_number(handle: &str) -> i64 {
    handle
        .bytes()
        .fold(7_i64, |acc, b| {
            acc.wrapping_mul(31).wrapping_add(i64::from(b))
        })
        .rem_euclid(1_000_000_000)
        .saturating_add(1)
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><style>{STYLE}</style></head>\
         <body><main><h1>{title}</h1>{body}</main></body></html>"
    )
}

/// Deliberately unlike the auth server's own styling.
///
/// Somebody demonstrating this should never be in doubt about which
/// side of the redirect they are looking at, and matching the real
/// chrome would make the mock harder to tell from the thing it stands
/// in for.
const STYLE: &str = "
:root { color-scheme: light; }
body { margin: 0; display: grid; place-items: center; min-height: 100vh;
       background: #fffbe6; color: #1a1a1a;
       font: 15px/1.5 ui-sans-serif, system-ui, sans-serif; }
main { width: min(30rem, 92vw); padding: 2rem; background: #fff;
       border: 2px dashed #c9a227; border-radius: 10px; }
h1 { margin: 0 0 .75rem; font-size: 1.35rem; }
p { margin: 0 0 .9rem; }
.warn { padding: .6rem .8rem; background: #fff3bf; border-radius: 6px;
        font-weight: 600; }
.meta { color: #666; font-size: .85rem; }
code { font-family: ui-monospace, monospace; font-size: .85em; }
.cast { display: grid; gap: .5rem; margin-top: 1rem; }
.cast form { margin: 0; }
.cast button { display: flex; flex-direction: column; align-items: flex-start;
               width: 100%; padding: .7rem .9rem; font: inherit; text-align: left;
               background: #fff; border: 1px solid #d8d8d8; border-radius: 8px;
               cursor: pointer; }
.cast button:hover { border-color: #1a1a1a; }
.cast span { color: #666; font-size: .85rem; }
.invent { margin-top: 1.25rem; border-top: 1px solid #eee; padding-top: 1rem; }
.invent label { display: block; margin-bottom: .35rem; color: #666;
                font-size: .85rem; }
.invent .row { display: flex; gap: .5rem; }
.invent input { flex: 1; min-width: 0; padding: .6rem .7rem; font: inherit;
                border: 1px solid #d8d8d8; border-radius: 8px; }
.invent button { padding: .6rem 1rem; font: inherit; background: #1a1a1a;
                 color: #fff; border: 0; border-radius: 8px; cursor: pointer; }
";

fn escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn urlencode(raw: &str) -> String {
    raw.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                String::from(char::from(b))
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{accounts, escape, urlencode};

    #[test]
    fn every_provider_has_a_cast_and_unknown_ones_have_none() {
        for provider in ["github", "google", "tone3000"] {
            assert!(!accounts(provider).is_empty(), "{provider}");
        }
        assert!(accounts("okta").is_empty());
    }

    #[test]
    fn a_github_id_is_numeric_because_the_parser_accepts_a_number() {
        // The real GitHub sends a number. A mock that sent a string
        // would leave that branch of `fetch_profile` untested.
        for account in accounts("github") {
            assert!(account.id.parse::<i64>().is_ok(), "{}", account.id);
        }
    }

    #[test]
    fn an_unknown_handle_becomes_an_account_so_parallel_tests_do_not_collide() {
        let invented = super::account("google", "e2e-7f3a").expect("invented");
        assert_eq!(invented.login, "e2e-7f3a");
        assert_eq!(invented.email, "e2e-7f3a@google.local");
        // TONE3000 has no addresses to invent.
        assert_eq!(
            super::account("tone3000", "e2e-7f3a")
                .expect("invented")
                .email,
            ""
        );
        assert!(super::account("okta", "e2e-7f3a").is_none());
        assert!(super::account("google", "   ").is_none());
    }

    #[test]
    fn an_invented_github_handle_still_gets_a_numeric_id() {
        let number = super::stable_number("e2e-7f3a");
        assert!(number > 0);
        assert_eq!(number, super::stable_number("e2e-7f3a"));
        assert_ne!(number, super::stable_number("e2e-7f3b"));
    }

    #[test]
    fn a_tone3000_id_is_a_uuid_and_carries_no_address() {
        for account in accounts("tone3000") {
            assert!(uuid::Uuid::parse_str(&account.id).is_ok(), "{}", account.id);
            assert!(account.email.is_empty(), "TONE3000 returns no address");
        }
    }

    #[test]
    fn markup_from_a_query_string_cannot_escape_its_attribute() {
        assert_eq!(
            escape(r#"" onload="alert(1)"#),
            "&quot; onload=&quot;alert(1)"
        );
        assert_eq!(escape("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn a_state_with_separators_survives_the_redirect() {
        assert_eq!(urlencode("a&b=c d"), "a%26b%3Dc%20d");
    }
}
