//! Islands: an extension's views drawn live on a share's now-playing view
//! (studio#445), in the page's extension sandbox, with the share page's
//! own CSP unchanged.
//!
//! An island is a WASI component a node operator linked in
//! ([`SiteIsland`], given to the node by a distribution): its files
//! (`architect-ext build-web`'s folder) are served by the node from a
//! node-owned path, never from a vault or a link. The page's player
//! loads the web runtime (`features/extensions/web`, served beside it),
//! which fetches the component's files itself and hands them to a
//! sandbox frame as bytes; the component's views are drawn into the
//! now-playing view's slots by `view.js` under the share pages' rules.
//!
//! **The sandbox frame is its own document** ([`sandbox_document`]),
//! served with its own policy ([`sandbox_headers`]): `sandbox
//! allow-scripts` (an opaque origin even opened at the top level or
//! framed without the attribute: no cookies, no storage), the runtime's
//! `SANDBOX_CSP` (its one script by hash, `blob:` workers, wasm, nothing
//! to fetch or connect to), `frame-ancestors 'self'`, `nosniff`, cached
//! immutably under a path named by its content. A `srcdoc` frame would
//! inherit the share page's CSP, which allows no wasm; a document of its
//! own leaves the share's policy exactly as it is. It is static and the
//! same for everyone.
//!
//! **A web view** ([`SiteIsland::web`]): the same views built as a plain
//! web bundle (wasm-bindgen's `--target web` glue, its modules and its
//! wasm, files of the island's folder named `web-*`), for a browser
//! without JS Promise Integration (Safari before 27, every browser on iOS
//! before it). Each view runs in a frame of its own, the
//! [`webview_document`], served with [`webview_headers`]: `sandbox
//! allow-scripts` (an opaque origin), its one boot script by hash, `blob:`
//! modules and wasm, inline style (its components' `style` attributes),
//! images and fonts only from `blob:`/`data:`, and nothing to fetch or
//! connect to. The page fetches the bundle's files from the node, checks
//! each against its digest ([`page_json`]'s `web.files`), and hands them
//! in as bytes with its theme and where the song is; the frame sends back
//! only the view's asks (the island's intents, checked by the page), its
//! height and that it is ready.

use std::borrow::Cow;
use std::sync::OnceLock;

use crate::assets::fingerprint;

/// Where the node serves islands, the runtime and the sandbox: a path no
/// site or link uses (`_` segments are the node's).
pub const ISLANDS_PATH: &str = "/_ext/";

/// The web runtime's sources, as the package ships them.
const RUNNER: &str = include_str!("../../extensions/web/runner.js");
const RUNTIME: &[(&str, &str)] = &[
    ("index.js", include_str!("../../extensions/web/index.js")),
    ("runner.js", RUNNER),
    ("check.js", include_str!("../../extensions/web/check.js")),
    ("host.js", include_str!("../../extensions/web/host.js")),
    ("view.js", include_str!("../../extensions/web/view.js")),
    (
        "sanitize.js",
        include_str!("../../extensions/web/sanitize.js"),
    ),
    ("world.js", include_str!("../../extensions/web/world.js")),
];
/// A web view frame's one script (`assets/webview.js`).
const WEBVIEW_SCRIPT: &str = include_str!("../assets/webview.js");

/// `instance.js`, which the runtime starts each instance's worker from
/// (as `scripts/build.mjs` writes `dist/instance-source.js`).
const INSTANCE: &str = include_str!("../../extensions/web/instance.js");

/// One extension's island: its id, its built web folder's files, and its
/// views for the now-playing view's slots.
///
/// Built once ([`new`](Self::new) and its setters); what it serves under
/// (its folder path, its lock's and its stylesheet's digests) is worked
/// out once, on first use.
#[derive(Debug, Clone)]
pub struct SiteIsland {
    id: String,
    files: Vec<(String, Cow<'static, [u8]>)>,
    progress: Option<String>,
    transport: Option<String>,
    lyrics: Option<String>,
    stylesheet: Option<String>,
    web: Option<String>,
    /// The operator's grant of WebRTC ([`Self::grant_webrtc`]).
    webrtc_granted: bool,
    derived: OnceLock<Derived>,
}

#[derive(Debug, Clone)]
struct Derived {
    folder: String,
    integrity: Option<String>,
    stylesheet_integrity: Option<String>,
}

fn sha256(bytes: &[u8]) -> String {
    use base64::Engine as _;
    use sha2::Digest as _;
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes))
    )
}

/// Whether `name`'s extension is exactly `ext` (the page's player checks
/// the same, case and all).
fn ext_is(name: &str, ext: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| e == ext)
}

/// Whether `name` is a web bundle's file: `web-` and a plain name (no
/// folder: its modules import each other as `./<name>`).
fn web_name(name: &str) -> bool {
    name.starts_with("web-") && !name.contains('/') && file_name_ok(name)
}

/// Whether `name` is a file path of a folder: segments of letters,
/// digits, `.`, `_`, `-`, none `.`, `..` or empty, no leading `/`.
fn file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

impl SiteIsland {
    /// Island `id` of `files`, no views yet. Its web bundle is found by its
    /// files: the one `web-*.js` that is not a `web-snippet-*.js` is its
    /// entry ([`Self::web`] names another).
    #[must_use]
    pub fn new(id: impl Into<String>, files: Vec<(String, Cow<'static, [u8]>)>) -> Self {
        let mut entries = files.iter().map(|(name, _)| name.as_str()).filter(|name| {
            web_name(name) && ext_is(name, "js") && !name.starts_with("web-snippet-")
        });
        let web = match (entries.next(), entries.next()) {
            (Some(one), None) => Some(one.to_owned()),
            _ => None,
        };
        Self {
            id: id.into(),
            files,
            progress: None,
            transport: None,
            lyrics: None,
            stylesheet: None,
            web,
            webrtc_granted: false,
            derived: OnceLock::new(),
        }
    }

    /// Its views as a web bundle too: `entry`, the wasm-bindgen glue of
    /// its folder (`web-<name>.js`, which exports `mount(view)`), with the
    /// rest of its `web-*` files (its modules and its one `.wasm`). A page
    /// in a browser without JS Promise Integration draws its views from it
    /// in [`webview_document`] frames. Anything but a `web-*.js` file of
    /// its folder is not used.
    #[must_use]
    pub fn web(mut self, entry: impl Into<String>) -> Self {
        let entry = entry.into();
        self.web = (web_name(&entry) && ext_is(&entry, "js") && self.file(&entry).is_some())
            .then_some(entry);
        self
    }

    /// Grant it WebRTC in its web view (the operator's grant): it gets the
    /// web view document that allows peer connections, if its extension
    /// also declares `capabilities.webrtc = ["peer-connections"]`. What it
    /// exposes: the visitor's IP to its peers and to the STUN/TURN servers
    /// it names, and a data channel out of the sandbox no policy meters.
    #[must_use]
    pub const fn grant_webrtc(mut self) -> Self {
        self.webrtc_granted = true;
        self
    }

    /// Whether its extension declares WebRTC (`capabilities.webrtc`
    /// holding `peer-connections`, in its folder's `architect.json`).
    #[must_use]
    pub fn declares_webrtc(&self) -> bool {
        self.file("architect.json")
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
            .and_then(|json| {
                json.pointer("/manifest/capabilities/webrtc")
                    .and_then(serde_json::Value::as_array)
                    .map(|list| list.iter().any(|v| v == "peer-connections"))
            })
            .unwrap_or(false)
    }

    /// Which web view its views run in: WebRTC only when declared and
    /// granted, never anything a page, a note or a message says.
    #[must_use]
    pub fn webrtc(&self) -> WebRtc {
        if self.webrtc_granted && self.declares_webrtc() {
            WebRtc::Granted
        } else {
            WebRtc::Blocked
        }
    }

    /// Its web bundle's files: its `web-*.js` and `web-*.wasm`, by name.
    fn web_files(&self) -> Vec<(&str, &[u8])> {
        if self.web.is_none() {
            return Vec::new();
        }
        self.files
            .iter()
            .filter(|(name, _)| web_name(name) && (ext_is(name, "js") || ext_is(name, "wasm")))
            .map(|(name, bytes)| (name.as_str(), bytes.as_ref()))
            .collect()
    }

    /// Its view `view` at the top of the now-playing view.
    #[must_use]
    pub fn progress(mut self, view: impl Into<String>) -> Self {
        self.progress = Some(view.into());
        self
    }

    /// Its view `view` under the page's chart.
    #[must_use]
    pub fn transport(mut self, view: impl Into<String>) -> Self {
        self.transport = Some(view.into());
        self
    }

    /// Its view `view` in the now-playing view's lyrics (a phone shell's
    /// Lyrics tab), the whole height the words had. Drawn only from its web
    /// bundle ([`Self::web`], in a web view frame the page sizes): given
    /// `lyrics` (`[{at, text}]`, the song's timed lyrics list as the page
    /// has it) and `artist` besides what the other views are. The page's
    /// own list shows until its first frame, for a song with no timed
    /// lines, and for good if the frame fails.
    #[must_use]
    pub fn lyrics(mut self, view: impl Into<String>) -> Self {
        self.lyrics = Some(view.into());
        self
    }

    /// `file` of its folder as its views' stylesheet (a path inside its
    /// folder: no `..`, no leading `/`; anything else is not used).
    #[must_use]
    pub fn stylesheet(mut self, file: impl Into<String>) -> Self {
        let file = file.into();
        self.stylesheet = file_name_ok(&file).then_some(file);
        self.derived = OnceLock::new();
        self
    }

    /// Its id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether its id is one a path may carry.
    #[must_use]
    pub fn id_ok(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 64
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && !self.id.starts_with('.')
    }

    fn derived(&self) -> &Derived {
        self.derived.get_or_init(|| {
            let mut all = Vec::new();
            for (path, bytes) in &self.files {
                all.extend_from_slice(path.as_bytes());
                all.push(0);
                all.extend_from_slice(bytes);
            }
            Derived {
                folder: format!("{ISLANDS_PATH}island/{}.{}/", self.id, fingerprint(&all)),
                integrity: self.file("architect.lock.json").map(sha256),
                stylesheet_integrity: self
                    .stylesheet
                    .as_deref()
                    .and_then(|s| self.file(s))
                    .map(sha256),
            }
        })
    }

    /// Its folder's path: `/_ext/island/<id>.<content hash>/`.
    #[must_use]
    pub fn folder(&self) -> &str {
        &self.derived().folder
    }

    /// `sha256-…` of its folder's lock (`architect.lock.json`, which names
    /// every file's digest): what the runtime's `loadExtension` pins it
    /// by. None without a lock (the runtime then refuses to load it).
    #[must_use]
    pub fn integrity(&self) -> Option<&str> {
        self.derived().integrity.as_deref()
    }

    /// `sha256-…` of its stylesheet: what the page's player pins it by.
    #[must_use]
    pub fn stylesheet_integrity(&self) -> Option<&str> {
        self.derived().stylesheet_integrity.as_deref()
    }

    /// File `rest` of its folder.
    #[must_use]
    pub fn file(&self, rest: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(path, _)| path == rest)
            .map(|(_, bytes)| bytes.as_ref())
    }
}

/// The sandbox frame's script, as the runtime names it (`runner.js`
/// `SANDBOX_SCRIPT`).
fn sandbox_script() -> &'static str {
    static SCRIPT: OnceLock<String> = OnceLock::new();
    SCRIPT.get_or_init(|| {
        between(RUNNER, "export const SANDBOX_SCRIPT = `", "`;").unwrap_or_default()
    })
}

/// The runtime's `SANDBOX_CSP`, with its hash put in.
fn sandbox_csp() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(|| {
        let hash = between(RUNNER, "export const SANDBOX_HASH = '", "';").unwrap_or_default();
        between(RUNNER, "export const SANDBOX_CSP = `", "`;")
            .unwrap_or_default()
            .replace("${SANDBOX_HASH}", &hash)
    })
}

fn between(text: &str, from: &str, to: &str) -> Option<String> {
    let start = text.find(from)?.checked_add(from.len())?;
    let rest = text.get(start..)?;
    Some(rest.get(..rest.find(to)?)?.to_owned())
}

/// The sandbox frame's document: its one script, nothing else. The same
/// bytes for everyone.
#[must_use]
pub fn sandbox_document() -> &'static str {
    static DOC: OnceLock<String> = OnceLock::new();
    DOC.get_or_init(|| format!("<!doctype html><script>{}</script>", sandbox_script()))
}

/// Where the sandbox document is served: named by its content, so it is
/// cached for good.
#[must_use]
pub fn sandbox_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        format!(
            "{ISLANDS_PATH}sandbox.{}.html",
            fingerprint(sandbox_document().as_bytes())
        )
    })
}

/// The sandbox document's own headers: its CSP (`sandbox allow-scripts`,
/// the runtime's `SANDBOX_CSP`, `frame-ancestors 'self'`), `nosniff`,
/// immutable.
#[must_use]
pub fn sandbox_headers() -> [(&'static str, String); 4] {
    [
        (
            "content-security-policy",
            format!(
                "sandbox allow-scripts; {}; frame-ancestors 'self'",
                sandbox_csp()
            ),
        ),
        ("x-content-type-options", "nosniff".to_owned()),
        (
            "cache-control",
            "public, max-age=31536000, immutable".to_owned(),
        ),
        ("content-type", "text/html; charset=utf-8".to_owned()),
    ]
}

/// Which of the two web view documents an island's views run in.
///
/// By default WebRTC is blocked (by policy and by its script, before any
/// bundle code runs); an island whose
/// extension declares `capabilities.webrtc = ["peer-connections"]` and
/// whose operator grants it ([`SiteIsland::grant_webrtc`]) gets the one
/// that allows it. Each has its own path, named by its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebRtc {
    /// The constructors removed by its script (the control, on every
    /// engine; its policy's `webrtc 'block'` is forward-looking only, no
    /// engine enforces it): the default.
    Blocked,
    /// The constructors left (`webrtc 'allow'`): declared and granted,
    /// and served only on a node where some island is.
    Granted,
}

impl WebRtc {
    const fn index(self) -> usize {
        match self {
            Self::Blocked => 0,
            Self::Granted => 1,
        }
    }
}

/// The web view's script for `rtc` (`assets/webview.js`; the granted one
/// with its `RTC` switch on).
fn webview_script(rtc: WebRtc) -> &'static str {
    static SCRIPTS: OnceLock<[String; 2]> = OnceLock::new();
    let all = SCRIPTS.get_or_init(|| {
        [
            WEBVIEW_SCRIPT.to_owned(),
            WEBVIEW_SCRIPT.replacen("var RTC = false;", "var RTC = true;", 1),
        ]
    });
    all.get(rtc.index()).map_or(WEBVIEW_SCRIPT, String::as_str)
}

/// A web view frame's policy (with `sandbox allow-scripts` and
/// `frame-ancestors 'self'` in [`webview_headers`]).
///
/// Blocked by the policy: every fetch, XHR, WebSocket, `EventSource`,
/// WebTransport, beacon and `<a ping>` (`connect-src 'none'`); scripts but
/// its own by hash and `blob:` modules (`script-src`: no speculation rules,
/// no remote import); images and fonts but `blob:`/`data:` (so no CSS
/// `url()` leaves it); media, workers, objects, manifests, `<base>`,
/// forms; `blob:`, `data:` and the web view documents as child frames
/// (`frame-src 'none'`; an `about:blank` or `srcdoc` child is held by the
/// sandbox's new opaque origin and the `script-src` it inherits). WebRTC is
/// stopped by its script, which removes the constructors on every engine;
/// `webrtc 'block'` is kept as forward-looking only (no engine enforces a
/// `webrtc` directive). By the sandbox flags (only `allow-scripts`): popups
/// (`window.open`), top navigation, forms, modals, same-origin. Its own
/// navigation away is held by the share page's policy (`frame-src`
/// through `default-src 'self'`): it may go only to the node. DNS
/// prefetch and preconnect are refused by `x-dns-prefetch-control: off`.
fn webview_csp(rtc: WebRtc) -> String {
    format!(
        "default-src 'none'; script-src '{}' blob: 'wasm-unsafe-eval'; style-src 'unsafe-inline'; img-src blob: data:; font-src blob: data:; connect-src 'none'; media-src 'none'; worker-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; webrtc '{}'",
        sha256(webview_script(rtc).as_bytes()),
        match rtc {
            WebRtc::Blocked => "block",
            WebRtc::Granted => "allow",
        }
    )
}

/// A web view's frame: a root for the view (`#main`, where a Dioxus web
/// bundle mounts), a reset, and its one script. The same for everyone.
#[must_use]
pub fn webview_document() -> &'static str {
    webview_document_for(WebRtc::Blocked)
}

/// The web view document for `rtc`.
#[must_use]
pub fn webview_document_for(rtc: WebRtc) -> &'static str {
    static DOCS: OnceLock<[String; 2]> = OnceLock::new();
    let doc = |rtc: WebRtc| {
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>html,body{{margin:0;padding:0;background:transparent;overflow:hidden}}body{{font:14px/1.3 system-ui,-apple-system,sans-serif;color:var(--foreground,CanvasText)}}</style></head><body><div id=\"main\"></div><script>{}</script></body></html>",
            webview_script(rtc)
        )
    };
    let all = DOCS.get_or_init(|| [doc(WebRtc::Blocked), doc(WebRtc::Granted)]);
    all.get(rtc.index()).map_or("", String::as_str)
}

/// Where the web view document is served, named by its content.
#[must_use]
pub fn webview_path() -> &'static str {
    webview_path_for(WebRtc::Blocked)
}

/// Where the web view document for `rtc` is served: its own path
/// (`webview.<hash>.html`, `webview-rtc.<hash>.html`), so the two are
/// never confused, nor cached one for the other.
#[must_use]
pub fn webview_path_for(rtc: WebRtc) -> &'static str {
    static PATHS: OnceLock<[String; 2]> = OnceLock::new();
    let all = PATHS.get_or_init(|| {
        [WebRtc::Blocked, WebRtc::Granted].map(|rtc| {
            let name = match rtc {
                WebRtc::Blocked => "webview",
                WebRtc::Granted => "webview-rtc",
            };
            format!(
                "{ISLANDS_PATH}{name}.{}.html",
                fingerprint(webview_document_for(rtc).as_bytes())
            )
        })
    });
    all.get(rtc.index()).map_or("", String::as_str)
}

/// The web view document's own headers: [`webview_csp`] in a sandbox,
/// framed only by the node's own pages, no DNS prefetch, `nosniff`,
/// immutable.
#[must_use]
pub fn webview_headers() -> Vec<(&'static str, String)> {
    webview_headers_for(WebRtc::Blocked)
}

/// [`webview_headers`] for `rtc`.
#[must_use]
pub fn webview_headers_for(rtc: WebRtc) -> Vec<(&'static str, String)> {
    vec![
        (
            "content-security-policy",
            format!(
                "sandbox allow-scripts; {}; frame-ancestors 'self'",
                webview_csp(rtc)
            ),
        ),
        ("x-content-type-options", "nosniff".to_owned()),
        ("x-dns-prefetch-control", "off".to_owned()),
        (
            "cache-control",
            "public, max-age=31536000, immutable".to_owned(),
        ),
        ("content-type", "text/html; charset=utf-8".to_owned()),
    ]
}

/// The headers a document under [`ISLANDS_PATH`] is served with.
///
/// Every one of them (without its policy it would run with the page's
/// origin's): the sandbox's or a web view's. None for anything else (a
/// module, a component, a sheet).
#[must_use]
pub fn document_headers(path: &str) -> Option<Vec<(&'static str, String)>> {
    if path == sandbox_path() {
        Some(sandbox_headers().into())
    } else if path == webview_path_for(WebRtc::Blocked) {
        Some(webview_headers_for(WebRtc::Blocked))
    } else if path == webview_path_for(WebRtc::Granted) {
        Some(webview_headers_for(WebRtc::Granted))
    } else {
        None
    }
}

/// The runtime's files, with `dist/instance-source.js` made as its build
/// makes it.
fn runtime_files() -> &'static [(String, String)] {
    static FILES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    FILES.get_or_init(|| {
        let mut files: Vec<(String, String)> = RUNTIME
            .iter()
            .map(|(name, body)| ((*name).to_owned(), (*body).to_owned()))
            .collect();
        let source = serde_json::to_string(INSTANCE).unwrap_or_default();
        files.push((
            "dist/instance-source.js".to_owned(),
            format!("// Generated from instance.js: do not edit.\nexport default {source};\n"),
        ));
        files
    })
}

/// Where the runtime is served: `/_ext/runtime.<content hash>/`, its
/// modules importing each other relatively under it.
#[must_use]
pub fn runtime_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut all = Vec::new();
        for (name, body) in runtime_files() {
            all.extend_from_slice(name.as_bytes());
            all.extend_from_slice(body.as_bytes());
        }
        format!("{ISLANDS_PATH}runtime.{}/", fingerprint(&all))
    })
}

/// Runtime file `rest` (a path under [`runtime_path`]).
#[must_use]
pub fn runtime_file(rest: &str) -> Option<&'static str> {
    runtime_files()
        .iter()
        .find(|(name, _)| name == rest)
        .map(|(_, body)| body.as_str())
}

/// What the node answers at `path` under [`ISLANDS_PATH`]: the body, its
/// content type and its cache policy. None for anything else.
#[must_use]
pub fn serve<'a>(islands: &'a [SiteIsland], path: &str) -> Option<(Cow<'a, [u8]>, &'static str)> {
    if path == sandbox_path() {
        return Some((
            Cow::Borrowed(sandbox_document().as_bytes()),
            "text/html; charset=utf-8",
        ));
    }
    // The document that allows WebRTC only on a node where some island is
    // granted it (and declares it): elsewhere it is not there at all.
    let granted = islands
        .iter()
        .any(|i| i.id_ok() && i.webrtc() == WebRtc::Granted);
    for rtc in [WebRtc::Blocked, WebRtc::Granted] {
        if rtc == WebRtc::Granted && !granted {
            continue;
        }
        if path == webview_path_for(rtc) {
            return Some((
                Cow::Borrowed(webview_document_for(rtc).as_bytes()),
                "text/html; charset=utf-8",
            ));
        }
    }
    if let Some(rest) = path.strip_prefix(runtime_path()) {
        return runtime_file(rest).map(|body| {
            (
                Cow::Borrowed(body.as_bytes()),
                "text/javascript; charset=utf-8",
            )
        });
    }
    for island in islands.iter().filter(|i| i.id_ok()) {
        if let Some(rest) = path.strip_prefix(island.folder()) {
            let body = island.file(rest)?;
            return Some((Cow::Borrowed(body), content_type(rest)));
        }
    }
    None
}

fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// What a share page carries for its player (`player.js`): the runtime,
/// the sandbox, and each island's folder and views, as JSON for
/// `<script type="application/json" id="site-islands">`.
#[must_use]
pub fn page_json(islands: &[SiteIsland]) -> serde_json::Value {
    let list: Vec<serde_json::Value> = islands
        .iter()
        .filter(|i| i.id_ok())
        .map(|i| {
            serde_json::json!({
                "id": i.id,
                "folder": i.folder(),
                "integrity": i.integrity(),
                "progress": i.progress,
                "transport": i.transport,
                // Its lyrics view: a web bundle's only.
                "lyrics": i.web.as_ref().and(i.lyrics.as_ref()),
                // Only with its digest: the player pins it as it pins the
                // component's files.
                "stylesheet": i.stylesheet_integrity().and(i.stylesheet.as_deref()),
                "stylesheetIntegrity": i.stylesheet_integrity(),
                // Its web bundle: each file pinned by its digest, as the
                // node holds it.
                "web": i.web.as_deref().map(|entry| serde_json::json!({
                    "entry": entry,
                    // Its frame's document: chosen here, by its declaration
                    // and its grant.
                    "webview": webview_path_for(i.webrtc()),
                    "files": i
                        .web_files()
                        .into_iter()
                        .map(|(name, bytes)| (name.to_owned(), serde_json::Value::String(sha256(bytes))))
                        .collect::<serde_json::Map<_, _>>(),
                })),
            })
        })
        .collect();
    serde_json::json!({
        "runtime": runtime_path(),
        "sandbox": sandbox_path(),
        "webview": webview_path(),
        "islands": list,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_sandbox_document_is_the_runtimes_script_under_its_own_policy() {
        let doc = sandbox_document();
        assert!(doc.starts_with("<!doctype html><script>(() => {"));
        assert!(
            doc.contains("e.source !== parent"),
            "it takes its port only from its parent"
        );
        let headers = sandbox_headers();
        let csp = &headers[0].1;
        assert!(csp.starts_with("sandbox allow-scripts;"), "{csp}");
        assert!(
            !csp.contains("allow-same-origin")
                && !csp.contains("allow-top-navigation")
                && !csp.contains("allow-popups")
                && !csp.contains("allow-forms")
                && !csp.contains("allow-modals")
        );
        assert!(csp.contains("connect-src 'none'") && csp.contains("'wasm-unsafe-eval'"));
        assert!(csp.contains("frame-ancestors 'self'"));
        assert!(
            csp.contains("script-src 'sha256-") && !csp.contains("${"),
            "{csp}"
        );
        assert!(
            !csp.contains("'self'; ") || csp.ends_with("frame-ancestors 'self'"),
            "the frame names no origin but its ancestors'"
        );
        assert_eq!(headers[1], ("x-content-type-options", "nosniff".to_owned()));
        assert!(headers[2].1.contains("immutable"));
        let path = sandbox_path();
        assert!(
            path.starts_with("/_ext/sandbox.")
                && Path::new(path).extension().is_some_and(|e| e == "html")
        );
    }

    /// A web view's frame (no JS Promise Integration needed): its own
    /// document, sandboxed, its one script by hash, nothing to fetch or
    /// connect to; the only widening over the sandbox's is inline style.
    #[test]
    fn a_web_view_frame_runs_its_bundle_under_its_own_policy() {
        let doc = webview_document();
        assert!(doc.contains("<div id=\"main\"></div>"));
        assert!(
            doc.contains("e.source !== parent"),
            "its port only from its parent"
        );
        let headers = webview_headers();
        let csp = &headers[0].1;
        assert!(csp.starts_with("sandbox allow-scripts;"), "{csp}");
        for no in [
            "allow-same-origin",
            "allow-top-navigation",
            "allow-popups",
            "allow-forms",
            "allow-modals",
            "'unsafe-eval'",
            "'self' blob",
            "https:",
            "http:",
            "*",
        ] {
            assert!(!csp.contains(no), "{no} in {csp}");
        }
        for yes in [
            "default-src 'none'",
            "connect-src 'none'",
            "worker-src 'none'",
            "frame-src 'none'",
            "img-src blob: data:",
            "style-src 'unsafe-inline'",
            "'wasm-unsafe-eval'",
            "frame-ancestors 'self'",
        ] {
            assert!(csp.contains(yes), "{yes} not in {csp}");
        }
        // The hash is the script the document carries, byte for byte.
        let script = doc
            .split("<script>")
            .nth(1)
            .and_then(|s| s.split("</script>").next())
            .unwrap();
        assert!(csp.contains(&format!("script-src '{}' blob:", sha256(script.as_bytes()))));
        assert!(webview_path().starts_with("/_ext/webview.") && ext_is(webview_path(), "html"));
        // WebRTC blocked by the policy and removed by the script; no DNS
        // prefetch; no child frame (a bundle cannot get the constructors
        // back from one).
        assert!(
            csp.contains("webrtc 'block'")
                && csp.contains("frame-src 'none'")
                && csp.contains("child-src 'none'")
        );
        assert!(script.contains("var RTC = false;") && script.contains("RTCPeerConnection"));
        assert!(headers.contains(&("x-dns-prefetch-control", "off".to_owned())));
        assert!(document_headers(webview_path()).is_some());
        assert!(document_headers(sandbox_path()).is_some());
        assert!(document_headers("/_ext/webview.html").is_none());
        assert_eq!(
            serve(&[], webview_path()).map(|(b, t)| (b.into_owned(), t)),
            Some((doc.as_bytes().to_vec(), "text/html; charset=utf-8"))
        );
    }

    /// WebRTC is a declared and granted capability: the default frame
    /// blocks it; an island declaring `capabilities.webrtc` and granted by
    /// its operator gets its own document that allows it; one that is not
    /// declared gets nothing from a grant, and nothing else chooses.
    #[test]
    fn webrtc_is_declared_and_granted_never_asked_for() {
        let manifest = |caps: &str| {
            format!(
                "{{\"module\":\"x.js\",\"manifest\":{{\"id\":\"demo.call\",\"capabilities\":{caps}}}}}"
            )
        };
        let island = |caps: &str| {
            SiteIsland::new(
                "demo.call",
                vec![
                    (
                        "architect.json".to_owned(),
                        Cow::Owned(manifest(caps).into_bytes()),
                    ),
                    (
                        "web-call.js".to_owned(),
                        Cow::Borrowed(b"export default 1".as_slice()),
                    ),
                    (
                        "web-call_bg.wasm".to_owned(),
                        Cow::Borrowed(b"\0asm".as_slice()),
                    ),
                ],
            )
            .progress("call")
        };
        let declared = r#"{"webrtc":["peer-connections"]}"#;
        assert_eq!(
            island(declared).webrtc(),
            WebRtc::Blocked,
            "declared, not granted"
        );
        assert_eq!(
            island("{}").grant_webrtc().webrtc(),
            WebRtc::Blocked,
            "granted, not declared"
        );
        assert_eq!(
            island(r#"{"webrtc":["camera"]}"#).grant_webrtc().webrtc(),
            WebRtc::Blocked
        );
        let granted = island(declared).grant_webrtc();
        assert_eq!(granted.webrtc(), WebRtc::Granted);
        let json = page_json(&[granted, island(declared)]);
        assert_eq!(
            json["islands"][0]["web"]["webview"],
            webview_path_for(WebRtc::Granted)
        );
        assert_eq!(
            json["islands"][1]["web"]["webview"],
            webview_path_for(WebRtc::Blocked)
        );
        // Two documents, two paths, each served with its own policy.
        assert_ne!(
            webview_path_for(WebRtc::Granted),
            webview_path_for(WebRtc::Blocked)
        );
        assert!(webview_path_for(WebRtc::Granted).starts_with("/_ext/webview-rtc."));
        // The document that allows WebRTC is served only where some island
        // is granted it: elsewhere a direct GET finds nothing.
        let rtc = webview_path_for(WebRtc::Granted);
        assert!(serve(&[], rtc).is_none());
        assert!(serve(&[island(declared), island("{}").grant_webrtc()], rtc).is_none());
        assert!(serve(&[island(declared).grant_webrtc()], rtc).is_some());
        assert!(
            serve(&[], webview_path_for(WebRtc::Blocked)).is_some(),
            "the default always"
        );
        let csp = |rtc| {
            document_headers(webview_path_for(rtc)).unwrap()[0]
                .1
                .clone()
        };
        assert!(
            csp(WebRtc::Granted).contains("webrtc 'allow'")
                && csp(WebRtc::Granted).contains("connect-src 'none'")
        );
        assert!(webview_document_for(WebRtc::Granted).contains("var RTC = true;"));
        assert!(csp(WebRtc::Blocked).contains("webrtc 'block'"));
        assert!(webview_document_for(WebRtc::Blocked).contains("var RTC = false;"));
        // A note cannot hand the page an islands list of its own: what it
        // draws loses every script element.
        let drawn = crate::sanitize::sanitize(
            "<script type=\"application/json\" id=\"site-islands\">{}</script><p>x</p>",
        );
        assert!(
            !drawn.contains("site-islands") && !drawn.contains("<script"),
            "{drawn}"
        );
    }

    /// An island's web bundle: its `web-*` modules and wasm, each with its
    /// digest; an entry that is not one of its `web-*.js` is not used.
    #[test]
    fn an_islands_web_bundle_is_its_web_files_pinned_by_digest() {
        let files = vec![
            (
                "web-x.js".to_owned(),
                Cow::Borrowed(b"export default 1".as_slice()),
            ),
            (
                "web-x_bg.wasm".to_owned(),
                Cow::Borrowed(b"\0asm".as_slice()),
            ),
            (
                "web-snippet-0.js".to_owned(),
                Cow::Borrowed(b"export const a = 1".as_slice()),
            ),
            ("x.wasm".to_owned(), Cow::Borrowed(b"\0asm2".as_slice())),
            ("island.css".to_owned(), Cow::Borrowed(b".a{}".as_slice())),
        ];
        let island = SiteIsland::new("studio.transport", files.clone())
            .progress("progress")
            .web("web-x.js");
        let json = page_json(std::slice::from_ref(&island));
        assert_eq!(json["webview"], webview_path());
        let web = &json["islands"][0]["web"];
        assert_eq!(web["entry"], "web-x.js");
        let mut names: Vec<&str> = web["files"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["web-snippet-0.js", "web-x.js", "web-x_bg.wasm"]);
        assert_eq!(web["files"]["web-x.js"], sha256(b"export default 1"));
        // Found by its files: the one entry that is no snippet.
        let found = SiteIsland::new("x", files.clone());
        assert_eq!(
            page_json(std::slice::from_ref(&found))["islands"][0]["web"]["entry"],
            "web-x.js"
        );
        let mut two = files.clone();
        two.push(("web-y.js".to_owned(), Cow::Borrowed(b"".as_slice())));
        assert!(
            page_json(&[SiteIsland::new("x", two)])["islands"][0]["web"].is_null(),
            "two entries: none"
        );
        for bad in [
            "x.wasm",
            "island.css",
            "web-nope.js",
            "../web-x.js",
            "web-x_bg.wasm",
        ] {
            let island = SiteIsland::new("x", files.clone()).web(bad);
            assert!(page_json(&[island])["islands"][0]["web"].is_null(), "{bad}");
        }
    }

    #[test]
    fn the_runtime_and_an_island_are_served_by_path_and_nothing_else() {
        let island = SiteIsland::new(
            "demo.counter",
            vec![
                ("extension.json".to_owned(), Cow::Borrowed(b"{}".as_slice())),
                ("x.wasm".to_owned(), Cow::Borrowed(b"\0asm".as_slice())),
            ],
        )
        .progress("bar");
        let islands = [island.clone()];
        let at = |p: &str| serve(&islands, p).map(|(b, t)| (b.into_owned(), t));
        assert_eq!(
            at(&format!("{}x.wasm", island.folder())),
            Some((b"\0asm".to_vec(), "application/wasm"))
        );
        assert!(at(&format!("{}index.js", runtime_path())).is_some());
        assert!(
            at(&format!("{}dist/instance-source.js", runtime_path()))
                .unwrap()
                .0
                .starts_with(b"// Generated")
        );
        assert!(at(sandbox_path()).is_some());
        for nope in [
            "/_ext/island/demo.counter/x.wasm",
            "/_ext/sandbox.html",
            "/_ext/runtime/index.js",
            &format!("{}../x.wasm", island.folder()),
            &format!("{}nope", island.folder()),
        ] {
            assert!(at(nope).is_none(), "{nope}");
        }
        let bad = SiteIsland::new(
            "../x",
            vec![("a".to_owned(), Cow::Borrowed(b"a".as_slice()))],
        );
        assert!(!bad.id_ok());
        assert!(serve(std::slice::from_ref(&bad), &format!("{}a", bad.folder())).is_none());
        let json = page_json(&islands);
        assert_eq!(json["islands"][0]["progress"], "bar");
        // A lyrics view is a web bundle's: without one, none.
        let words = island.lyrics("words");
        assert!(page_json(std::slice::from_ref(&words))["islands"][0]["lyrics"].is_null());
        let web = SiteIsland::new(
            "demo.words",
            vec![(
                "web-x.js".to_owned(),
                Cow::Borrowed(b"export function mount(){}".as_slice()),
            )],
        )
        .lyrics("words")
        .web("web-x.js");
        assert_eq!(
            page_json(std::slice::from_ref(&web))["islands"][0]["lyrics"],
            "words"
        );
        // No stylesheet without a valid name and its digest.
        let files = vec![("a.css".to_owned(), Cow::Borrowed(b".a{}".as_slice()))];
        let sheet = SiteIsland::new("x", files.clone()).stylesheet("a.css");
        assert!(sheet.stylesheet_integrity().unwrap().starts_with("sha256-"));
        assert_eq!(
            page_json(std::slice::from_ref(&sheet))["islands"][0]["stylesheet"],
            "a.css"
        );
        for bad in [
            "../a.css",
            "/a.css",
            "a/../b.css",
            "",
            "a\\b.css",
            "./a.css",
        ] {
            let island = SiteIsland::new("x", files.clone()).stylesheet(bad);
            assert!(island.stylesheet_integrity().is_none(), "{bad}");
            assert!(
                page_json(&[island])["islands"][0]["stylesheet"].is_null(),
                "{bad}"
            );
        }
        // Worked out once.
        assert!(std::ptr::eq(sheet.folder(), sheet.folder()));
        assert_eq!(json["sandbox"], sandbox_path());
    }
}
