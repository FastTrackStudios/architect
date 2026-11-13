//! Architect in a browser (#431): the web app's shell, served with the
//! site and the docs as one static bundle.
//!
//! The site (`scripts/web-bundle.sh`, `just web`, the `architect-web`
//! image in `flake.nix`) has three parts on one host:
//!
//! | path | what |
//! | --- | --- |
//! | `/` | the site (`apps/site`, prerendered: the landing page, `/download`, `/changelog`, `/extensions`) |
//! | `/docs/` | the docs, `docs/content` built by `architect site build` |
//! | `/app/` | this app ([`APP`]); the sign-in comes back to `/auth/callback` ([`CALLBACK`]) |
//!
//! The app always lives at `/app/`, signed in or not; `/` stays the
//! site's landing page, which links to it. Everything else on the host (`/vox`,
//! `/org`, `/s`, …) belongs to the headless node, routed by the reverse
//! proxy in front of both (`deploy/chart/architect`).
//!
//! **Sign-in** is the issuer's redirect flow with PKCE
//! (`auth_client::oidc`): a public client named `architect`, which the
//! issuer registers for `architect.fasttrackstudio.app` (fts-auth's
//! `appDomains`). The verifier waits in `sessionStorage` while the browser
//! is at the issuer; the session is kept in `localStorage` and renewed
//! with its refresh token before it runs out.
//!
//! **Which distribution** it is (#491) comes from `/app/distribution.json`
//! ([`Distribution`]), what the distribution's binary printed when the
//! bundle was built (`cargo xtask dist web`): its name, and the extensions
//! compiled into it, which the app lists (the deployment's node, the same
//! binary running `serve`, draws their blocks on share links). Without
//! one it is the Architect app.
//!
//! **Settings** come from `/app/config.json` ([`Config`]), read at start,
//! so one bundle serves any host: the chart's `ConfigMap` replaces the
//! file with the deployment's issuer.
//!
//! **The demo** (`/app/?demo`, [`DEMO`], the landing page's Demo button) is
//! the app itself (`app`: the shell, the workspace, the navigator, ⌘K and
//! settings, the components the desktop and phone apps draw with, on
//! dioxus-web) on the demo vault ([`demo`]), in the tab's memory: no
//! sign-in, nothing sent anywhere, gone on reload.
//!
//! This module is the part that needs no browser (the config, where a
//! URL leads, what each screen shows), tested natively; `browser` (wasm32
//! only) does the storage, the requests and the DOM.

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod browser;
pub mod demo;
pub mod ext_host;
pub mod favourites;
pub mod keys;

#[cfg(target_arch = "wasm32")]
pub use browser::start;

use std::fmt::Write as _;

use serde::Deserialize;

/// Where the app is served.
pub const APP: &str = "/app/";
/// Where the issuer sends the browser back to, on the same origin: the
/// path every first-party client is registered with
/// (`https://<domain>/auth/callback`).
pub const CALLBACK: &str = "/auth/callback";
/// The scopes the app asks for: `offline_access` gets a refresh token,
/// so a sign-in lasts until it is revoked rather than an hour.
pub const SCOPE: &str = "openid email profile offline_access";
/// The node's status, over its HTTP face, on this origin
/// (`apps/desktop/src/node/service.rs` `Node`).
pub const NODE_STATUS: &str = "/node/status";
/// The demo's URL: the app on the demo vault, in memory.
pub const DEMO: &str = "/app/?demo";
/// Which distribution this bundle is (#491), beside the app.
pub const DISTRIBUTION: &str = "/app/distribution.json";
/// Where the desktop app is downloaded.
pub const DOWNLOADS: &str = "https://github.com/FastTrackStudios/architect/releases/latest";

/// `/app/config.json`: what differs between deployments.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    /// The OIDC issuer (`https://auth.fasttrackstudio.app`).
    pub issuer: String,
    /// The public client the issuer knows this app as.
    #[serde(default = "default_client")]
    pub client_id: String,
}

fn default_client() -> String {
    "architect".to_owned()
}

impl Config {
    /// Read `/app/config.json`.
    ///
    /// # Errors
    ///
    /// Not JSON, no `issuer`, or an issuer that is not an `http(s)` URL.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut config: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        config.issuer = config.issuer.trim().trim_end_matches('/').to_owned();
        if !(config.issuer.starts_with("https://") || config.issuer.starts_with("http://")) {
            return Err(format!("issuer `{}` is not an http(s) URL", config.issuer));
        }
        Ok(config)
    }

    /// The redirect URI for a page on `origin`.
    #[must_use]
    pub fn redirect_uri(origin: &str) -> String {
        format!("{}{CALLBACK}", origin.trim_end_matches('/'))
    }
}

/// `/app/distribution.json`: the distribution this bundle was built for
/// (`architect_distribution::Distribution::describe`), the fields the app
/// shows.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Distribution {
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// Its compiled-in extensions.
    #[serde(default)]
    pub extensions: Vec<BuiltIn>,
}

/// An extension compiled into the distribution.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BuiltIn {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// The bundle it came in.
    #[serde(default)]
    pub bundle: Option<String>,
}

impl Default for Distribution {
    /// The Architect app, with no built-ins listed: a bundle built
    /// without a `distribution.json`.
    fn default() -> Self {
        Self {
            name: "Architect".to_owned(),
            version: String::new(),
            extensions: Vec::new(),
        }
    }
}

impl Distribution {
    /// Read `/app/distribution.json`.
    ///
    /// # Errors
    ///
    /// Not JSON, or no `name`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let d: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if d.name.trim().is_empty() {
            return Err("distribution.json names no app".to_owned());
        }
        Ok(d)
    }

    /// Its compiled-in extensions as a card's section; nothing with none.
    #[must_use]
    pub fn built_in(&self) -> String {
        if self.extensions.is_empty() {
            return String::new();
        }
        let mut rows = String::new();
        for e in &self.extensions {
            let _ = write!(
                rows,
                "<li data-extension=\"{}\"><strong>{}</strong> <span class=\"muted\">{}</span><br><span class=\"muted\">{}</span></li>",
                escape(&e.id),
                escape(&e.name),
                escape(&e.version),
                escape(&e.description)
            );
        }
        format!("<section class=\"built-in\"><h3>Built in</h3><ul>{rows}</ul></section>")
    }
}

/// Where a URL leads, from its path and query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Back from the issuer with a code to redeem.
    Callback { code: String, state: String },
    /// Back from the issuer with a refusal (`error`, its description).
    Refused(String),
    /// The app itself.
    App,
    /// The app on the demo vault, in memory ([`DEMO`]).
    Demo,
}

/// Where `path` with `query` (without its `?`) leads.
#[must_use]
pub fn route(path: &str, query: &str) -> Route {
    let param = |name: &str| {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (k == name).then(|| decode(v))
        })
    };
    if path.trim_end_matches('/') != CALLBACK {
        return if param("demo").is_some() {
            Route::Demo
        } else {
            Route::App
        };
    }
    if let Some(error) = param("error") {
        return Route::Refused(param("error_description").unwrap_or(error));
    }
    match (param("code"), param("state")) {
        (Some(code), Some(state)) if !code.is_empty() => Route::Callback { code, state },
        _ => Route::Refused("the issuer sent no code".to_owned()),
    }
}

/// Percent-decode a query value (`+` is a space).
fn decode(v: &str) -> String {
    let bytes = v.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |at: usize| {
            bytes
                .get(at)
                .and_then(|h| char::from(*h).to_digit(16))
                .and_then(|d| u8::try_from(d).ok())
        };
        match b {
            b'%' => {
                if let (Some(hi), Some(lo)) = (hex(i.saturating_add(1)), hex(i.saturating_add(2))) {
                    out.push(hi.wrapping_mul(16).wrapping_add(lo));
                    i = i.saturating_add(3);
                    continue;
                }
                out.push(b);
            }
            b'+' => out.push(b' '),
            _ => out.push(b),
        }
        i = i.saturating_add(1);
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Escape text for HTML.
#[must_use]
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Who is signed in, as the issuer's `userinfo` says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Person {
    pub name: Option<String>,
    pub email: Option<String>,
}

/// The node's answer to [`NODE_STATUS`]; the fields the app shows.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NodeStatus {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub org: Option<String>,
    #[serde(default)]
    pub vaults: u32,
    #[serde(default)]
    pub notes: u32,
    #[serde(default)]
    pub devices: u32,
}

/// What a screen shows; each is drawn by [`view`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// Reading the config, or redeeming a code.
    Loading,
    /// Nobody signed in.
    SignedOut { problem: Option<String> },
    /// Signed in; the node's status once it answered (or why not).
    SignedIn {
        person: Person,
        node: Option<Result<NodeStatus, String>>,
    },
    /// The app cannot start (no usable config).
    Broken(String),
}

/// The HTML of `screen` in distribution `dist`, for the app's root.
/// Buttons carry `data-action` (`sign-in`, `sign-out`), which the browser
/// half wires.
#[must_use]
pub fn view(screen: &Screen, dist: &Distribution) -> String {
    let name = escape(&dist.name);
    let built_in = dist.built_in();
    match screen {
        Screen::Loading => "<p class=\"muted\">Loading…</p>".to_owned(),
        Screen::Broken(why) => format!(
            "<section class=\"card\"><h2>{name} can't start here</h2><p class=\"problem\">{}</p></section>",
            escape(why)
        ),
        Screen::SignedOut { problem } => {
            let problem = problem
                .as_deref()
                .map(|p| format!("<p class=\"problem\">{}</p>", escape(p)))
                .unwrap_or_default();
            format!(
                "<section class=\"card\"><h2>Sign in to {name}</h2>\
                 <p>Your notes, projects and files, on every device and in sync.</p>{problem}\
                 <p class=\"row\"><button class=\"primary\" data-action=\"sign-in\">Sign in</button>\
                 <a class=\"button\" href=\"{DEMO}\">Try the demo</a></p>{built_in}</section>"
            )
        }
        Screen::SignedIn { person, node } => {
            let who = person
                .name
                .as_deref()
                .or(person.email.as_deref())
                .unwrap_or("you");
            let email = person
                .email
                .as_deref()
                .filter(|e| Some(*e) != person.name.as_deref())
                .map(|e| format!(" <span class=\"muted\">{}</span>", escape(e)))
                .unwrap_or_default();
            let node = match node {
                None => "<p class=\"muted\">Asking the node…</p>".to_owned(),
                Some(Err(why)) => format!(
                    "<p class=\"problem\">The node did not answer: {}</p>",
                    escape(why)
                ),
                Some(Ok(s)) => format!(
                    "<dl class=\"facts\"><dt>Node</dt><dd>{}</dd><dt>Version</dt><dd>{}</dd>\
                     <dt>Org</dt><dd>{}</dd><dt>Vaults</dt><dd>{} ({} notes)</dd><dt>Devices</dt><dd>{}</dd></dl>",
                    escape(&s.name),
                    escape(&s.version),
                    escape(s.org.as_deref().unwrap_or("none yet")),
                    s.vaults,
                    s.notes,
                    s.devices
                ),
            };
            format!(
                "<section class=\"card\"><h2>Signed in as {}{email}</h2>{node}\
                 <p>The full app is the desktop app for now; its views come to the browser here.</p>\
                 <p class=\"row\"><a class=\"button primary\" href=\"{DOWNLOADS}\">Download the desktop app</a>\
                 <a class=\"button\" href=\"{DEMO}\">Open the demo</a>\
                 <a class=\"button\" href=\"/docs/\">Docs</a>\
                 <button data-action=\"sign-out\">Sign out</button></p>{built_in}</section>",
                escape(who)
            )
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_config_needs_an_issuer_and_defaults_the_client() {
        let c = Config::parse(r#"{"issuer":"https://auth.example/"}"#).unwrap();
        assert_eq!(c.issuer, "https://auth.example");
        assert_eq!(c.client_id, "architect");
        assert!(Config::parse(r#"{"issuer":"auth.example"}"#).is_err());
        assert!(Config::parse("{}").is_err());
        assert_eq!(
            Config::redirect_uri("https://architect.example/"),
            "https://architect.example/auth/callback"
        );
    }

    #[test]
    fn the_callback_is_read_and_everything_else_is_the_app() {
        assert_eq!(route("/app/", ""), Route::App);
        assert_eq!(route("/app/anything", "code=x"), Route::App);
        // The landing page's Demo button.
        assert_eq!(route("/app/", "demo"), Route::Demo);
        assert_eq!(route("/app/", "x=1&demo="), Route::Demo);
        assert_eq!(route("/app/", "demonstrate"), Route::App);
        assert!(matches!(route("/auth/callback", "demo"), Route::Refused(_)));
        assert_eq!(
            route("/auth/callback", "code=a%2Fb&state=s+1"),
            Route::Callback {
                code: "a/b".into(),
                state: "s 1".into()
            }
        );
        assert_eq!(
            route(
                "/auth/callback/",
                "error=access_denied&error_description=No%20thanks"
            ),
            Route::Refused("No thanks".into())
        );
        assert!(matches!(
            route("/auth/callback", "state=s"),
            Route::Refused(_)
        ));
    }

    #[test]
    fn screens_escape_what_they_show() {
        let html = view(
            &Screen::SignedIn {
                person: Person {
                    name: Some("<b>Ada</b>".into()),
                    email: Some("ada@x.test".into()),
                },
                node: Some(Ok(NodeStatus {
                    name: "home".into(),
                    version: "0.30.0".into(),
                    org: None,
                    vaults: 2,
                    notes: 9,
                    devices: 3,
                })),
            },
            &Distribution::default(),
        );
        assert!(html.contains("&lt;b&gt;Ada&lt;/b&gt;"), "{html}");
        assert!(html.contains("ada@x.test"));
        assert!(html.contains("2 (9 notes)"));
        assert!(html.contains("data-action=\"sign-out\""));
        assert!(
            view(
                &Screen::SignedOut { problem: None },
                &Distribution::default()
            )
            .contains("data-action=\"sign-in\"")
        );
    }

    #[test]
    fn a_distribution_names_the_app_and_lists_what_is_compiled_in() {
        let d = Distribution::parse(
            r#"{"name":"Studio","version":"0.12.0","extensions":[{"id":"studio-setlists","name":"Set<lists>","version":"0.1.0","description":"Songs.","bundle":null}],"bundles":[]}"#,
        )
        .unwrap();
        let html = view(&Screen::SignedOut { problem: None }, &d);
        assert!(html.contains("Sign in to Studio"), "{html}");
        assert!(
            html.contains("data-extension=\"studio-setlists\""),
            "{html}"
        );
        assert!(html.contains("Set&lt;lists&gt;"), "escaped: {html}");
        let plain = view(
            &Screen::SignedOut { problem: None },
            &Distribution::default(),
        );
        assert!(plain.contains("Sign in to Architect"));
        assert!(!plain.contains("built-in"), "none listed: {plain}");
        assert!(Distribution::parse(r#"{"name":" "}"#).is_err());
    }
}
