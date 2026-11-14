//! The `web` view kind (#222): an extension's own HTML, CSS and JS in a pane.
//!
//! A native web view on a desk (wry; `WKWebView` on iOS), a sandboxed
//! `<iframe>` in a browser, and the bridge that is its only way into the
//! app (`docs/content/architecture/embedded-views.md`, "Web views")
//!
//! This module is what every host of the kind shares, with no web view
//! in it, so it builds for wasm and is tested without a window:
//!
//! - **Its files** ([`serve`]): a view's `src` and everything it loads
//!   come from the extension's package over the [`SCHEME`] protocol
//!   (`architect-ext://<extension>/<path>`), never from disk paths or the
//!   network: a path outside the package is refused, and every document
//!   is served under [`csp`], which allows nothing off the package (no
//!   `connect-src`: the view's network is the host's `fetch`, checked
//!   against `capabilities.hosts`).
//! - **The bridge's messages** ([`Inbound`], [`Outbound`]): the view asks
//!   with a host-service request (`crate::hostcall::HostRequest`, the same
//!   JSON every runtime sends) and the host answers it after
//!   `crate::hostcall::check`, through the extension's
//!   `hostapi::ExtensionApi` on a desk or the browser's `web::WebHost`;
//!   the host pushes the events the view listens for ([`Listening`]), the
//!   theme ([`theme_css`]) and whether it is shown.
//! - **The view's runtime** ([`RUNTIME_JS`]): the script every host puts
//!   in the view before its own, `window.architect`, whichever transport
//!   it is in (wry's `ipc`, `WKWebView`'s message handler, a frame's port).
//!   The typed SDK (`@fasttrackstudio/architect/webview`) is a thin layer
//!   over it.
//! - **Keys and overlays** ([`hands_back`], [`Shown`]): typed keys stay in
//!   the view but an Escape, which gives the focus back to the app; a
//!   native view is hidden while one of the app's overlays (⌘K, a menu, a
//!   popover) is open, as nothing the app draws can be over it.
//!
//! The view's code is untrusted ([`architect_workspace::Capability::Untrusted`]):
//! it is never given a capability its extension was not granted, and a
//! web view extension is a `data` package, its code running only inside
//! the view.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The custom protocol a desk's web view loads a package's files from:
/// `architect-ext://<extension id>/<path in the package>`.
pub const SCHEME: &str = "architect-ext";

/// The largest message a view may send the host: bigger ones are dropped
/// with an answer saying why.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// How many calls a view may have waiting on the host at once: past it,
/// a call is answered `unsupported` at once.
pub const MAX_IN_FLIGHT: usize = 64;

/// The URL a desk's web view opens for `path` in extension `id`'s
/// package.
#[must_use]
pub fn url(id: &str, path: &str) -> String {
    format!("{SCHEME}://{id}/{}", path.trim_start_matches('/'))
}

/// The content security policy every document of a web view is served
/// under.
///
/// Its own package only: scripts, styles, images, fonts, media and
/// workers from `'self'` (and `blob:`/`data:` it makes), WebAssembly
/// compiled from them; nothing over the network (`connect-src 'none'`:
/// its fetches go through the host's `fetch`, limited to
/// `capabilities.hosts`); no frames, objects, forms, `<base>` or
/// navigation away. In a browser the frame is
/// `sandbox="allow-scripts"` as well, an opaque origin.
#[must_use]
pub const fn csp() -> &'static str {
    "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; \
     worker-src 'self' blob:; connect-src 'none'; frame-src 'none'; child-src 'self' blob:; \
     object-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'"
}

/// A file of a package, as the protocol answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub body: Vec<u8>,
    pub mime: &'static str,
    /// Its headers: the [`csp`] on every one, `nosniff`, no caching.
    pub headers: Vec<(&'static str, String)>,
}

/// Why the protocol did not answer with a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// A path outside the package (`..`, an absolute path, a link out).
    Outside(String),
    /// Nothing there.
    Missing(String),
    /// Another extension's package, from this one's view.
    Foreign { own: String, asked: String },
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Outside(p) => write!(f, "{p} is outside the package"),
            Self::Missing(p) => write!(f, "{p} is not in the package"),
            Self::Foreign { own, asked } => {
                write!(f, "a web view of {own} may not load {asked}'s files")
            }
        }
    }
}

impl std::error::Error for AssetError {}

impl AssetError {
    /// The HTTP status the protocol answers it with.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Outside(_) | Self::Foreign { .. } => 403,
            Self::Missing(_) => 404,
        }
    }
}

/// A file's media type, by its extension.
#[must_use]
pub fn mime(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The headers a package's file is served with.
#[must_use]
pub fn headers(mime: &'static str) -> Vec<(&'static str, String)> {
    vec![
        ("content-type", mime.to_owned()),
        ("content-security-policy", csp().to_owned()),
        ("x-content-type-options", "nosniff".to_owned()),
        ("cache-control", "no-store".to_owned()),
        ("referrer-policy", "no-referrer".to_owned()),
    ]
}

/// `path` (a URL's path, `/dist/index.html`) inside package folder `dir`:
/// the file it names, or why not. A path that steps out (`..`, a root, a
/// drive), or a link that leads out, is refused; `/` is `index.html`.
///
/// # Errors
///
/// [`AssetError::Outside`] or [`AssetError::Missing`].
pub fn resolve(dir: &Path, path: &str) -> Result<PathBuf, AssetError> {
    let clean = path.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode(clean).ok_or_else(|| AssetError::Outside(path.to_owned()))?;
    let asked = decoded.trim_start_matches('/');
    let rel = if asked.is_empty() || asked.ends_with('/') {
        format!("{asked}index.html")
    } else {
        asked.to_owned()
    };
    if rel.contains('\\')
        || !Path::new(&rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(AssetError::Outside(path.to_owned()));
    }
    let file = dir.join(&rel);
    // A link inside the package that points out of it is outside too.
    match (file.canonicalize(), dir.canonicalize()) {
        (Ok(found), Ok(top)) if found.starts_with(&top) && found.is_file() => Ok(found),
        (Ok(found), Ok(top)) if !found.starts_with(&top) => Err(AssetError::Outside(rel)),
        _ => Err(AssetError::Missing(rel)),
    }
}

/// Serves `url` (one of [`SCHEME`]'s, or its path alone) for a web view of
/// extension `own`, whose package is `dir`.
///
/// # Errors
///
/// The path is outside the package, names another extension's, or there
/// is nothing there.
pub fn serve(own: &str, dir: &Path, url: &str) -> Result<Asset, AssetError> {
    let path = match url.strip_prefix(&format!("{SCHEME}://")) {
        Some(rest) => {
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            if host != own {
                return Err(AssetError::Foreign {
                    own: own.to_owned(),
                    asked: host.to_owned(),
                });
            }
            path
        }
        None => url,
    };
    let file = resolve(dir, path)?;
    let body = std::fs::read(&file).map_err(|_| AssetError::Missing(path.to_owned()))?;
    let mime = mime(&file.to_string_lossy());
    Ok(Asset {
        body,
        mime,
        headers: headers(mime),
    })
}

fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            out.push(u8::try_from(hi.checked_mul(16)?.checked_add(lo)?).ok()?);
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).ok()
}

// ── The bridge's messages ───────────────────────────────────────────

/// What a view says to its host.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Inbound {
    /// A host-service call (`crate::hostcall::HostRequest`'s JSON),
    /// answered with [`Outbound::Answer`] of the same `id`.
    Call { id: u64, req: Value },
    /// The view's document is ready: the host sends it the theme and
    /// whether it is shown.
    Ready,
    /// The events it wants pushed ([`Outbound::Event`]): names, or
    /// `<prefix>.*`, in place of the ones before.
    Listen { events: Vec<String> },
    /// An Escape the view did not take: the app takes the focus back.
    Escape,
}

/// What a host says to its view.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Outbound {
    /// The answer to call `id`: `crate::hostcall::HostAnswer`'s JSON
    /// (`{ok}` or `{error}`).
    Answer { id: u64, answer: Value },
    /// An event it listens for, with its detail.
    Event { name: String, detail: Value },
    /// The app's theme: its tokens as CSS custom properties on `:root`
    /// ([`theme_css`]) and its scheme, `light` or `dark`.
    Theme { scheme: String, css: String },
    /// Whether it is shown (its tab is the one in front, nothing of the
    /// app's is over it).
    Shown { shown: bool },
}

impl Outbound {
    /// As the text the transport carries.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Reads a view's message.
///
/// # Errors
///
/// It is too big ([`MAX_MESSAGE`]) or not one of [`Inbound`]'s; the
/// reason is for the view's log. A malformed call still names its id
/// when it has one, so the view's promise is answered.
pub fn parse(text: &str) -> Result<Inbound, (Option<u64>, String)> {
    if text.len() > MAX_MESSAGE {
        return Err((
            None,
            format!(
                "a message of {} bytes is over the bridge's {MAX_MESSAGE}",
                text.len()
            ),
        ));
    }
    serde_json::from_str(text).map_err(|e| {
        let id = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|v| v.get("id").and_then(Value::as_u64));
        (id, format!("not a bridge message: {e}"))
    })
}

/// The answer to call `id` that refuses it before the host sees it.
#[must_use]
pub fn refusal(id: u64, why: &str) -> Outbound {
    Outbound::Answer {
        id,
        answer: serde_json::json!({ "error": { "kind": "unsupported", "what": why } }),
    }
}

/// The events a view asked to hear ([`Inbound::Listen`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listening(Vec<String>);

impl Listening {
    #[must_use]
    pub const fn new(events: Vec<String>) -> Self {
        Self(events)
    }

    /// Whether it hears `name` (`doc.saved` under `doc.*`; `*` is every
    /// one).
    #[must_use]
    pub fn hears(&self, name: &str) -> bool {
        self.0.iter().any(|p| crate::events::covers(p, name))
    }
}

// ── Theme ───────────────────────────────────────────────────────────

/// The app's theme as a style sheet for a view.
///
/// Each token is a custom
/// property on `:root` (`--brand: #6c5ce7`; an app's dotted token
/// `studio.accent` is `--studio-accent`), and its `color-scheme`, so form
/// controls and scroll bars follow light and dark. A token whose name is
/// not `a-z 0-9 - .`, or whose value could end the rule, is left out.
#[must_use]
pub fn theme_css<'a>(scheme: &str, tokens: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let scheme = if scheme == "light" { "light" } else { "dark" };
    let mut css = format!(":root{{color-scheme:{scheme};");
    for (name, value) in tokens {
        let name = name.trim_start_matches("--").replace('.', "-");
        let ok_name = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        let ok_value = !value.is_empty()
            && !value.contains(['{', '}', ';', '<', '>', '\\', '\n', '\r'])
            && !value.to_ascii_lowercase().contains("url(");
        if ok_name && ok_value {
            css.push_str("--");
            css.push_str(&name);
            css.push(':');
            css.push_str(value.trim());
            css.push(';');
        }
    }
    css.push('}');
    css
}

/// The view's runtime: `window.architect`.
///
/// Put in every native web view
/// before its own scripts (wry's initialisation script; `WKWebView`'s user
/// script), and in a sandboxed frame by the app's own bundle (the npm
/// package's `webview-runtime.js`, the same file). It installs the small
/// component sheet every view gets (`.arch-button`, `.arch-input`,
/// `.arch-card`, `.arch-list`, `.arch-badge`, in the theme's tokens) before
/// the view's own.
pub const RUNTIME_JS: &str = include_str!("webview/runtime.js");

/// The script a native host puts in a view before its own.
#[must_use]
pub fn boot_script() -> String {
    RUNTIME_JS.to_owned()
}

// ── Keys and overlays ───────────────────────────────────────────────

/// Whether a key the view did not take hands the focus back to the app:
/// Escape, with no modifier. Every other key stays in the view, the
/// app's chords included (a web view is its own keyboard's).
#[must_use]
pub fn hands_back(key: &str, modifiers: bool) -> bool {
    key == "Escape" && !modifiers
}

/// Whether a native web view is shown.
///
/// Its tab in front, its pane on
/// screen, and none of the app's overlays open (⌘K, a menu, a popover,
/// a dialog), which the app draws in its own document and a native view
/// would cover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// Its tab is the one in front.
    pub front: bool,
    /// Its pane has a size on screen.
    pub sized: bool,
    /// How many of the app's overlays are open.
    pub overlays: u32,
}

impl Shown {
    #[must_use]
    pub const fn shown(self) -> bool {
        self.front && self.sized && self.overlays == 0
    }
}

// ── The bridge on a desk ────────────────────────────────────────────

#[cfg(feature = "host")]
pub use desk::Bridge;

#[cfg(feature = "host")]
mod desk {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use futures::future::BoxFuture;

    use super::{Inbound, Listening, MAX_IN_FLIGHT, Outbound, parse, refusal};
    use crate::host::Host;
    use crate::hostapi::ExtensionApi;

    /// What sends a message into the view (wry's `evaluate_script`,
    /// `WKWebView`'s): called from any thread.
    pub type Post = Arc<dyn Fn(Outbound) + Send + Sync>;

    /// One web view's bridge on a native host.
    ///
    /// Its messages in, answered
    /// as its extension through [`ExtensionApi`] (every call checked by
    /// `crate::hostcall::check` against what it was granted), and the
    /// events it listens for out, heard through [`Host::hear`].
    pub struct Bridge {
        api: ExtensionApi,
        host: Host,
        post: Post,
        listening: Arc<Mutex<Listening>>,
        heard: u64,
        in_flight: Arc<AtomicUsize>,
        on_escape: Arc<dyn Fn() + Send + Sync>,
    }

    impl Bridge {
        /// The bridge of a view of extension `id` on `host`, posting into
        /// the view with `post`; `on_escape` gives the app the focus back.
        #[must_use]
        pub fn new(
            host: &Host,
            id: &str,
            post: Post,
            on_escape: impl Fn() + Send + Sync + 'static,
        ) -> Self {
            let listening = Arc::new(Mutex::new(Listening::default()));
            let heard = {
                let listening = Arc::clone(&listening);
                let post = Arc::clone(&post);
                host.hear(
                    id,
                    Arc::new(move |name, detail| {
                        let hears = listening
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .hears(name);
                        if hears {
                            post(Outbound::Event {
                                name: name.to_owned(),
                                detail: detail.clone(),
                            });
                        }
                    }),
                )
            };
            Self {
                api: host.api(id),
                host: host.clone(),
                post,
                listening,
                heard,
                in_flight: Arc::new(AtomicUsize::new(0)),
                on_escape: Arc::new(on_escape),
            }
        }

        /// The extension whose view it is.
        #[must_use]
        pub fn id(&self) -> &str {
            self.api.id()
        }

        /// Takes a message from the view. A call's answer is posted when
        /// the returned work, run on the app's executor, is done; the
        /// rest are done at once (`None`).
        pub fn receive(&self, text: &str) -> Option<BoxFuture<'static, ()>> {
            let message = match parse(text) {
                Ok(m) => m,
                Err((id, why)) => {
                    self.host
                        .logs()
                        .push(self.id(), crate::logs::Level::Warn, "webview", why.clone());
                    if let Some(id) = id {
                        (self.post)(refusal(id, &why));
                    }
                    return None;
                }
            };
            match message {
                Inbound::Call { id, req } => {
                    if self.in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
                        self.in_flight.fetch_sub(1, Ordering::SeqCst);
                        (self.post)(refusal(
                            id,
                            &format!("more than {MAX_IN_FLIGHT} calls are waiting on the host"),
                        ));
                        return None;
                    }
                    let api = self.api.clone();
                    let post = Arc::clone(&self.post);
                    let in_flight = Arc::clone(&self.in_flight);
                    Some(Box::pin(async move {
                        let text = api.call_json(&req.to_string()).await;
                        let answer = serde_json::from_str(&text).unwrap_or_default();
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        post(Outbound::Answer { id, answer });
                    }))
                }
                Inbound::Listen { events } => {
                    *self
                        .listening
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = Listening::new(events);
                    None
                }
                Inbound::Escape => {
                    (self.on_escape)();
                    None
                }
                Inbound::Ready => None,
            }
        }

        /// Sends the view the theme or whether it is shown.
        pub fn send(&self, message: Outbound) {
            (self.post)(message);
        }
    }

    impl Drop for Bridge {
        fn drop(&mut self) {
            self.host.unhear(self.heard);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::create_dir_all(dir.path().join("dist/assets")).unwrap();
        std::fs::write(dir.path().join("dist/index.html"), "<p>hi</p>").unwrap();
        std::fs::write(dir.path().join("dist/assets/app.js"), "1").unwrap();
        std::fs::write(dir.path().join("index.html"), "root").unwrap();
        dir
    }

    #[test]
    fn a_package_serves_its_own_files_under_the_policy() {
        let dir = package();
        let a = serve("demo.web", dir.path(), &url("demo.web", "dist/index.html")).unwrap();
        assert_eq!(a.body, b"<p>hi</p>");
        assert_eq!(a.mime, "text/html; charset=utf-8");
        let policy = &a.headers.iter().find(|(k, _)| *k == "content-security-policy").unwrap().1;
        assert!(policy.contains("connect-src 'none'"), "{policy}");
        assert!(policy.contains("default-src 'none'"), "{policy}");
        let js = serve("demo.web", dir.path(), "/dist/assets/app.js?v=3").unwrap();
        assert_eq!(js.mime, "text/javascript; charset=utf-8");
        assert_eq!(serve("demo.web", dir.path(), "/").unwrap().body, b"root");
    }

    #[test]
    fn nothing_outside_the_package_is_served() {
        let dir = package();
        let outside = dir.path().parent().unwrap().join("secret.txt");
        let _ = std::fs::write(&outside, "no");
        for path in ["/../secret.txt", "/dist/%2e%2e/%2e%2e/secret.txt", "/dist/..%2f..%2fsecret.txt", "/dist\\..\\..\\secret.txt"] {
            let e = serve("demo.web", dir.path(), path).unwrap_err();
            assert!(matches!(e, AssetError::Outside(_)), "{path}: {e:?}");
            assert_eq!(e.status(), 403);
        }
        assert_eq!(serve("demo.web", dir.path(), "/nope.js").unwrap_err().status(), 404);
        // A root is read inside the package, never the disk's.
        assert_eq!(serve("demo.web", dir.path(), "//etc/passwd").unwrap_err().status(), 404);
        let e = serve("demo.web", dir.path(), &url("other.ext", "index.html")).unwrap_err();
        assert!(matches!(e, AssetError::Foreign { .. }), "{e:?}");
        let _ = std::fs::remove_file(outside);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_package_is_outside() {
        let dir = package();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("key"), "k").unwrap();
        std::os::unix::fs::symlink(other.path().join("key"), dir.path().join("dist/key")).unwrap();
        let e = serve("demo.web", dir.path(), "/dist/key").unwrap_err();
        assert!(matches!(e, AssetError::Outside(_)), "{e:?}");
    }

    #[test]
    fn messages_are_plain_json() {
        let m = parse(r#"{"t":"call","id":7,"req":{"op":"run","action":"APP_STATUS"}}"#).unwrap();
        assert!(matches!(m, Inbound::Call { id: 7, .. }));
        assert_eq!(parse(r#"{"t":"escape"}"#).unwrap(), Inbound::Escape);
        let (id, why) = parse(r#"{"t":"call","id":9}"#).unwrap_err();
        assert_eq!(id, Some(9), "{why}");
        let big = format!(r#"{{"t":"ready","pad":"{}"}}"#, "x".repeat(MAX_MESSAGE));
        assert!(parse(&big).unwrap_err().1.contains("over the bridge"));
        let out = Outbound::Event { name: "doc.saved".into(), detail: serde_json::json!({"path": "a.md"}) };
        assert_eq!(out.to_json(), r#"{"t":"event","name":"doc.saved","detail":{"path":"a.md"}}"#);
    }

    #[test]
    fn a_view_hears_what_it_listens_for() {
        let l = Listening::new(vec!["doc.*".into(), "app:playhead.changed".into()]);
        assert!(l.hears("doc.saved"));
        assert!(l.hears("app:playhead.changed"));
        assert!(!l.hears("app.focused"));
        assert!(!Listening::default().hears("doc.saved"));
    }

    #[test]
    fn the_theme_is_custom_properties_and_nothing_else() {
        let css = theme_css(
            "light",
            [
                ("brand", "#6c5ce7"),
                ("--ink", "rgb(1 2 3)"),
                ("studio.accent", "red"),
                ("bad name", "red"),
                ("evil", "red;}body{display:none"),
                ("img", "url(https://x/y.png)"),
            ],
        );
        assert_eq!(css, ":root{color-scheme:light;--brand:#6c5ce7;--ink:rgb(1 2 3);--studio-accent:red;}");
        assert!(theme_css("dark", []).contains("color-scheme:dark"));
        assert!(RUNTIME_JS.contains(".arch-button"));
        assert!(RUNTIME_JS.contains("window.architect"));
        // The npm package ships the same runtime.
        assert_eq!(RUNTIME_JS, include_str!("../web/webview-runtime.js"));
    }

    #[test]
    fn the_sdk_names_every_host_call() {
        // features/extensions/web/webview.d.ts types every op the host has.
        let dts = include_str!("../web/webview.d.ts");
        // Every name `HostRequest::op` gives, read from its match.
        let source = include_str!("hostcall.rs");
        let body = source
            .split_once("pub const fn op(&self) -> &'static str {")
            .and_then(|(_, rest)| rest.split_once("\n    }\n"))
            .map_or("", |(body, _)| body);
        let ops: Vec<&str> = body
            .split("=> \"")
            .skip(1)
            .filter_map(|rest| rest.split_once('"').map(|(op, _)| op))
            .collect();
        assert!(ops.len() > 40, "{ops:?}");
        for op in ops {
            assert!(dts.contains(&format!("op: \"{op}\"")), "webview.d.ts has no {op}");
        }
    }

    #[test]
    fn escape_alone_gives_the_focus_back() {
        assert!(hands_back("Escape", false));
        assert!(!hands_back("Escape", true));
        assert!(!hands_back("k", false));
    }

    #[test]
    fn an_overlay_hides_a_native_view() {
        let s = Shown { front: true, sized: true, overlays: 0 };
        assert!(s.shown());
        assert!(!Shown { overlays: 1, ..s }.shown());
        assert!(!Shown { front: false, ..s }.shown());
        assert!(!Shown { sized: false, ..s }.shown());
    }
}
