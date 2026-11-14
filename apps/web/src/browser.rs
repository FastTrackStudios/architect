//! The browser half: storage, requests, the DOM. Everything it decides
//! is the crate root's ([`crate::route`], [`crate::view`]); this only
//! carries it out.

// Nothing is `Send` in a browser.
#![allow(clippy::future_not_send)]

use std::cell::RefCell;
use std::rc::Rc;

use auth_client::oidc::{self, Pkce, Refresh};
use auth_client::{StoredSession, TokenStore, TokenStoreError};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, Request, RequestInit, Response, Storage, Window};

use crate::{
    APP, Config, DISTRIBUTION, Distribution, NODE_STATUS, NodeStatus, Person, Route, SCOPE, Screen,
    route, view,
};

/// The session, in `localStorage`.
const SESSION_KEY: &str = "architect.session";
/// The PKCE verifier and state while the browser is at the issuer, in
/// `sessionStorage` (one attempt, one tab).
const VERIFIER_KEY: &str = "architect.pkce.verifier";
const STATE_KEY: &str = "architect.pkce.state";

fn window() -> Result<Window, String> {
    web_sys::window().ok_or_else(|| "no window".to_owned())
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(js)`
fn js(e: JsValue) -> String {
    e.as_string()
        .or_else(|| {
            e.dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

fn local() -> Result<Storage, String> {
    window()?
        .local_storage()
        .map_err(js)?
        .ok_or_else(|| "no localStorage".to_owned())
}

fn session_storage() -> Result<Storage, String> {
    window()?
        .session_storage()
        .map_err(js)?
        .ok_or_else(|| "no sessionStorage".to_owned())
}

/// Unix seconds now.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)] // a time in ms, well inside i64
fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// The session in `localStorage`. Holds nothing itself (it asks the
/// browser each time), so it is `Send + Sync` as `TokenStore` wants.
struct LocalStore;

impl TokenStore for LocalStore {
    fn save(&self, session: &StoredSession) -> Result<(), TokenStoreError> {
        let text = serde_json::to_string(session)?;
        local()
            .and_then(|s| s.set_item(SESSION_KEY, &text).map_err(js))
            .map_err(TokenStoreError::Backend)
    }

    fn load(&self) -> Result<Option<StoredSession>, TokenStoreError> {
        let text = local()
            .and_then(|s| s.get_item(SESSION_KEY).map_err(js))
            .map_err(TokenStoreError::Backend)?;
        Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
    }

    fn clear(&self) -> Result<(), TokenStoreError> {
        local()
            .and_then(|s| s.remove_item(SESSION_KEY).map_err(js))
            .map_err(TokenStoreError::Backend)
    }
}

/// `method url` with `body` and `headers`: the status and the body.
async fn fetch(
    method: &str,
    url: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<(u16, String), String> {
    let init = RequestInit::new();
    init.set_method(method);
    if let Some(body) = body {
        init.set_body(&JsValue::from_str(body));
    }
    let request = Request::new_with_str_and_init(url, &init).map_err(js)?;
    for (name, value) in headers {
        request.headers().set(name, value).map_err(js)?;
    }
    let response: Response = JsFuture::from(window()?.fetch_with_request(&request))
        .await
        .map_err(js)?
        .dyn_into()
        .map_err(js)?;
    let text = JsFuture::from(response.text().map_err(js)?)
        .await
        .map_err(js)?
        .as_string()
        .unwrap_or_default();
    Ok((response.status(), text))
}

/// POST a form to the issuer's token endpoint; the body, whatever the
/// status (`auth_client::oidc` reads an error answer too).
async fn token_post(config: &Config, body: String) -> Result<String, String> {
    let url = format!("{}{}", config.issuer, oidc::TOKEN_PATH);
    fetch(
        "POST",
        &url,
        Some(&body),
        &[("content-type", "application/x-www-form-urlencoded")],
    )
    .await
    .map(|(_, text)| text)
}

thread_local! {
    /// The app's config, once read: what Sign out needs to revoke.
    static CONFIG: RefCell<Option<Config>> = const { RefCell::new(None) };
    /// Which distribution this bundle is (`/app/distribution.json`), read
    /// once at start.
    static DIST: RefCell<Distribution> = RefCell::new(Distribution::default());
}

fn draw(screen: &Screen) {
    if let Some(root) = window()
        .ok()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("app"))
    {
        DIST.with(|d| root.set_inner_html(&view(screen, &d.borrow())));
    }
}

/// The distribution's name (`/app/distribution.json`, read at start):
/// what the app calls itself.
pub fn app_name() -> String {
    DIST.with(|d| d.borrow().name.clone())
}

/// Reads `/app/distribution.json` into [`DIST`], and names the page and
/// its header after it. Missing, the bundle is the Architect app's.
async fn read_distribution() {
    let Ok((200, text)) = fetch("GET", DISTRIBUTION, None, &[]).await else {
        return;
    };
    let Ok(dist) = Distribution::parse(&text) else {
        return;
    };
    if let Some(doc) = window().ok().and_then(|w| w.document()) {
        doc.set_title(&dist.name);
        if let Ok(Some(brand)) = doc.query_selector("header .brand") {
            brand.set_text_content(Some(&dist.name));
        }
    }
    DIST.with(|d| *d.borrow_mut() = dist);
}

/// The page's start: the Architect app ([`architect_distribution::architect`]),
/// its first-party extensions linked in. A distribution crate's web build
/// turns feature `start` off and calls [`run`] with its own.
#[cfg(feature = "start")]
#[wasm_bindgen(start)]
pub fn start() {
    run(architect_distribution::architect());
}

/// The whole of a distribution's web build (#495), its counterpart of
/// `architect_desktop::run` and `architect_mobile::run`: `distribution`
/// is installed (its name, and the extensions it links, which the app's
/// host loads: `app::extensions`), then the app starts.
///
/// ```ignore
/// #[wasm_bindgen::prelude::wasm_bindgen(start)]
/// pub fn start() {
///     architect_web::run(architect_distribution::architect().named("Studio"));
/// }
/// ```
pub fn run(distribution: architect_distribution::Distribution) {
    // Once: the library's start (the bundle) and the program's `main`
    // (`dx serve`, src/main.rs) both come here in a dev build.
    thread_local!(static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    // A panic says why on the console (wasm's own trap says only
    // "unreachable").
    std::panic::set_hook(Box::new(|info| {
        let stack = js_sys::Reflect::get(&js_sys::Error::new(""), &"stack".into())
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_default();
        web_sys::console::error_1(&format!("architect-web panicked: {info}\n{stack}").into());
    }));
    let dist = distribution.install();
    if let Ok(described) = serde_json::from_value::<Distribution>(dist.describe()) {
        DIST.with(|d| *d.borrow_mut() = described);
    }
    spawn_local(async {
        if let Err(why) = open().await {
            draw(&Screen::Broken(why));
        }
    });
}

async fn open() -> Result<(), String> {
    let window = window()?;
    let location = window.location();
    let path = location.pathname().map_err(js)?;
    let search = location.search().map_err(js)?;
    let route = route(&path, search.trim_start_matches('?'));
    // The demo: the app at once, on the demo vault in memory, nothing
    // asked of the issuer or the node.
    // Which distribution this is, first: the demo is named for it too.
    read_distribution().await;
    if route == Route::Showcase {
        crate::demo::choose(crate::demo::Vault::Showcase);
    }
    if matches!(route, Route::Demo | Route::Showcase) {
        crate::app::mount(crate::app::Mode::Demo);
        return Ok(());
    }
    // A share link (`/s/<code>/…`, #646): the app as the link's guest,
    // nothing asked of the issuer. Its code stays in this page: the app
    // sends it only to this origin's node.
    if let Route::Share { code, rest } = &route {
        crate::app::mount(crate::app::Mode::Guest {
            code: code.clone(),
            rest: rest.clone(),
        });
        return Ok(());
    }
    draw(&Screen::Loading);
    let (status, text) = fetch("GET", &format!("{APP}config.json"), None, &[]).await?;
    if status != 200 {
        return Err(format!("{APP}config.json answered {status}"));
    }
    let config = Rc::new(Config::parse(&text)?);
    CONFIG.with(|c| *c.borrow_mut() = Some((*config).clone()));
    wire(Rc::clone(&config))?;
    let problem = match route {
        Route::App | Route::Demo | Route::Showcase | Route::Share { .. } => None,
        Route::Callback { code, state } => redeem(&config, &code, &state).await.err(),
        Route::Refused(why) => Some(why),
    };
    if path != APP {
        // Leave the callback (its code is spent) for the app's own URL.
        let _ = window
            .history()
            .and_then(|h| h.replace_state_with_url(&JsValue::NULL, "", Some(APP)));
    }
    match problem {
        Some(why) => draw(&Screen::SignedOut { problem: Some(why) }),
        None => show(&config).await,
    }
    Ok(())
}

/// Answer the buttons, by delegation on the app's root.
fn wire(config: Rc<Config>) -> Result<(), String> {
    let root = window()?
        .document()
        .and_then(|d| d.get_element_by_id("app"))
        .ok_or_else(|| "no #app".to_owned())?;
    let click = Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
        let action = event
            .target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|e| e.closest("[data-action]").ok().flatten())
            .and_then(|e| e.get_attribute("data-action"));
        match action.as_deref() {
            Some("sign-in") => {
                if let Err(why) = sign_in(&config) {
                    draw(&Screen::SignedOut { problem: Some(why) });
                }
            }
            Some("sign-out") => {
                let config = Rc::clone(&config);
                spawn_local(async move {
                    revoke(&config).await;
                    draw(&Screen::SignedOut { problem: None });
                });
            }
            _ => {}
        }
    });
    root.add_event_listener_with_callback("click", click.as_ref().unchecked_ref())
        .map_err(js)?;
    click.forget();
    Ok(())
}

/// Ends the session at the issuer and here, then shows the app signed
/// out (the account menu's Sign out, in the app).
pub fn sign_out() {
    spawn_local(async {
        if let Some(config) = CONFIG.with(|c| c.borrow().clone()) {
            revoke(&config).await;
        } else {
            let _ = LocalStore.clear();
        }
        if let Ok(w) = window() {
            let _ = w.location().assign(APP);
        }
    });
}

/// Revokes the stored sign-in's grant at the issuer (RFC 7009: its
/// refresh token and access tokens stop working, so a copy of either is
/// dead too), then forgets it here. Forgotten here even when the issuer
/// cannot be reached: the person asked to be signed out.
async fn revoke(config: &Config) {
    if let Ok(Some(session)) = LocalStore.load() {
        let token = session.refresh_token.as_deref().unwrap_or(&session.token);
        let body = oidc::revoke_request_body(&config.client_id, token);
        let _ = fetch(
            "POST",
            &format!("{}{}", config.issuer, oidc::REVOKE_PATH),
            Some(&body),
            &[("content-type", "application/x-www-form-urlencoded")],
        )
        .await;
    }
    let _ = LocalStore.clear();
}

/// Send the browser to the issuer, its verifier parked in this tab.
fn sign_in(config: &Config) -> Result<(), String> {
    let window = window()?;
    let crypto = window.crypto().map_err(js)?;
    let (mut verifier, mut state) = ([0u8; 32], [0u8; 16]);
    crypto
        .get_random_values_with_u8_array(&mut verifier)
        .map_err(js)?;
    crypto
        .get_random_values_with_u8_array(&mut state)
        .map_err(js)?;
    let pkce = Pkce::from_entropy(verifier, state);
    let parked = session_storage()?;
    parked.set_item(VERIFIER_KEY, pkce.verifier()).map_err(js)?;
    parked.set_item(STATE_KEY, pkce.state()).map_err(js)?;
    let origin = window.location().origin().map_err(js)?;
    let url = oidc::authorize_url(
        &config.issuer,
        &config.client_id,
        &Config::redirect_uri(&origin),
        &pkce,
        SCOPE,
    );
    window.location().assign(&url).map_err(js)
}

/// Redeem the code the issuer sent back, and keep the session.
async fn redeem(config: &Config, code: &str, state: &str) -> Result<(), String> {
    let parked = session_storage()?;
    let verifier = parked.get_item(VERIFIER_KEY).map_err(js)?;
    let expected = parked.get_item(STATE_KEY).map_err(js)?;
    let _ = parked.remove_item(VERIFIER_KEY);
    let _ = parked.remove_item(STATE_KEY);
    let (Some(verifier), Some(expected)) = (verifier, expected) else {
        return Err(
            "This sign-in was started in another tab, or ran out. Sign in again.".to_owned(),
        );
    };
    let pkce = Pkce::resume(verifier, expected);
    pkce.check_state(state).map_err(|e| e.to_string())?;
    let origin = window()?.location().origin().map_err(js)?;
    let body = oidc::token_request_body(
        &config.client_id,
        &Config::redirect_uri(&origin),
        code,
        &pkce,
    );
    let text = token_post(config, body).await?;
    let token = oidc::token_response_from(&text).map_err(|e| e.to_string())?;
    LocalStore
        .save(&token.into_session(now()))
        .map_err(|e| e.to_string())
}

/// The stored session, renewed first when it is about to run out.
async fn session(config: &Config) -> Option<StoredSession> {
    let presented = LocalStore.load().ok().flatten();
    let refreshed = oidc::refresh_if_needed(&LocalStore, &config.client_id, now(), |body| {
        token_post(config, body)
    })
    .await;
    match refreshed {
        Ok(Refresh::Fresh(s) | Refresh::Refreshed(s) | Refresh::NoRefreshToken(s)) => Some(s),
        Ok(Refresh::SignedOut) => None,
        Err(_) => {
            // A refresh token is single use: another tab that renewed
            // first left a newer session here. Not signed out then.
            let stored = LocalStore.load().ok().flatten();
            if let Some(stored) = stored
                .filter(|s| presented.as_ref().map(|p| &p.refresh_token) != Some(&s.refresh_token))
            {
                return Some(stored);
            }
            let _ = LocalStore.clear();
            None
        }
    }
}

/// Who the token belongs to; `Ok(None)` when the issuer refuses it.
async fn whoami(config: &Config, token: &str) -> Result<Option<Person>, String> {
    let bearer = format!("Bearer {token}");
    let (status, text) = fetch(
        "GET",
        &format!("{}/oauth2/userinfo", config.issuer),
        None,
        &[("authorization", &bearer)],
    )
    .await?;
    if status == 401 {
        return Ok(None);
    }
    let user = oidc::user_from(&text).map_err(|e| format!("{e} ({status})"))?;
    Ok(Some(Person {
        name: user.name,
        email: user.email,
    }))
}

async fn show(config: &Config) {
    let Some(mut session) = session(config).await else {
        draw(&Screen::SignedOut { problem: None });
        return;
    };
    let mut person = whoami(config, &session.token).await;
    if matches!(person, Ok(None)) {
        // Refused: renew once, then give up.
        if let Ok(Refresh::Refreshed(s)) =
            oidc::refresh_now(&LocalStore, &config.client_id, now(), |body| {
                token_post(config, body)
            })
            .await
        {
            session = s;
            person = whoami(config, &session.token).await;
        }
    }
    let person = match person {
        Ok(Some(p)) => p,
        Ok(None) => {
            let _ = LocalStore.clear();
            draw(&Screen::SignedOut {
                problem: Some("Your session ended. Sign in again.".to_owned()),
            });
            return;
        }
        // The issuer is unreachable: show what the session knows.
        Err(_) => Person {
            name: None,
            email: session.email.clone(),
        },
    };
    draw(&Screen::SignedIn {
        person: person.clone(),
        node: None,
        lane: None,
    });
    let node = node_status().await;
    // The node hosts the account's org: the app on its vaults.
    let lane = match node.as_ref().ok().and_then(|s| s.hosts.clone()) {
        Some(org) => match open_org(&org, &session.token).await {
            Ok(vault) => {
                if let Some(lane) = vault.org.clone() {
                    wasm_bindgen_futures::spawn_local(keep_lane_signed_in(
                        config.clone(),
                        lane,
                        session.clone(),
                    ));
                }
                let who = person
                    .name
                    .clone()
                    .or_else(|| person.email.clone())
                    .unwrap_or_default();
                crate::app::mount(crate::app::Mode::Org {
                    vault,
                    person: who,
                    org,
                });
                return;
            }
            Err(why) => Some(why),
        },
        None => None,
    };
    draw(&Screen::SignedIn {
        person,
        node: Some(node),
        lane,
    });
}

/// Renews the session before it runs out and dials the org's lane again
/// with each new token (the lane's bearer is fixed when it is dialled; an
/// expired one leaves the lane anonymous). Signed out, it says so.
async fn keep_lane_signed_in(
    config: Config,
    lane: crate::app::org::OrgVaults,
    mut session: StoredSession,
) {
    loop {
        let wait = session.expires_at_unix.map_or(300, |at| {
            at.saturating_sub(oidc::REFRESH_MARGIN_SECS)
                .saturating_sub(now())
                .clamp(15, 3_600)
        });
        let ms = u32::try_from(wait.saturating_mul(1_000)).unwrap_or(u32::MAX);
        gloo_timers::future::TimeoutFuture::new(ms).await;
        let Some(renewed) = self::session(&config).await else {
            lane.report("Your session ended: sign in again to keep editing.");
            return;
        };
        if renewed.token != session.token
            && let Err(why) = lane.redial(renewed.token.clone()).await
        {
            lane.report(format!("Your org's vaults did not reconnect: {why}"));
        }
        session = renewed;
    }
}

/// The org's vaults over its lane, listed, their texts read after.
async fn open_org(org: &str, token: &str) -> Result<crate::app::vault::WebVault, String> {
    let lane = crate::app::org::OrgVaults::open(org, token.to_owned()).await?;
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    // a time in ms, well inside i64
    let now = js_sys::Date::now() as i64;
    let (source, notes) = lane.list(org, now).await?;
    lane.fill(source.clone(), notes, now);
    Ok(crate::app::vault::WebVault {
        source,
        org: Some(lane),
    })
}

/// The node behind this host, over its HTTP face.
async fn node_status() -> Result<NodeStatus, String> {
    let (status, text) = fetch(
        "POST",
        NODE_STATUS,
        Some("{}"),
        &[("content-type", "application/json")],
    )
    .await?;
    if status != 200 {
        return Err(format!("{NODE_STATUS} answered {status}"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// The signed-in session's token in this browser, if there is one and it
/// has not run out: what a share link's guest calls carry on this origin
/// (#646), so a link for signed-in people opens for its member. Sent only
/// to this origin's node, the app's own.
pub fn stored_token() -> Option<String> {
    let session = LocalStore.load().ok().flatten()?;
    session
        .expires_at_unix
        .is_none_or(|at| at > now())
        .then_some(session.token)
}
