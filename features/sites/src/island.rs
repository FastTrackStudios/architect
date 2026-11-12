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
    stylesheet: Option<String>,
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
    /// Island `id` of `files`, no views yet.
    #[must_use]
    pub fn new(id: impl Into<String>, files: Vec<(String, Cow<'static, [u8]>)>) -> Self {
        Self {
            id: id.into(),
            files,
            progress: None,
            transport: None,
            stylesheet: None,
            derived: OnceLock::new(),
        }
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
                // Only with its digest: the player pins it as it pins the
                // component's files.
                "stylesheet": i.stylesheet_integrity().and(i.stylesheet.as_deref()),
                "stylesheetIntegrity": i.stylesheet_integrity(),
            })
        })
        .collect();
    serde_json::json!({
        "runtime": runtime_path(),
        "sandbox": sandbox_path(),
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
