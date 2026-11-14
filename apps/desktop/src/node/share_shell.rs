//! A share link's pages are the web app (#646).
//!
//! `/s/<code>/` and every page path below it answer the web app's own
//! page (`apps/web/app/index.html`, compiled in), which boots as a guest
//! of the link it was opened at, reads the link through its guest service
//! (`ShareGuest`) and draws it. The node renders no note to HTML for a
//! link any more.
//!
//! What the node adds to the app's page, and nothing else:
//!
//! - a `<head>` for link previews and crawlers: `<title>`, `description`
//!   and `og:*` from the link's title (only once the link is open to the
//!   caller: a signed-in link names nothing to anyone else), and `robots`
//!   `noindex` unless the link is public;
//! - one `<noscript>` line;
//! - the app's own Content-Security-Policy as a header ([`policy`]), the
//!   one `scripts/web-routes.md` gives and `apps/web/sws.toml` sends for
//!   `/app/`, with this node's issuer.
//!
//! The page loads the app from `/app/start.js` and `/app/style.css` on
//! the same origin: the web bundle's, served beside the node (production's
//! proxy: the bundle for everything the node does not own; `just dev`: the
//! web app's dev server, which writes its own `start.js`).
//!
//! Only a node told so serves that page ([`WebApp`], `ARCHITECT_WEB_APP`):
//! one with no web app beside it (the desktop's, Studio's, a self-hosted
//! node, the chart with `web.enabled=false`) answers a short page saying
//! the link opens in the web app, which this server does not serve
//! ([`answer`]), never a page that waits for a script that is not there.
//! The page carries the node's share-page contract
//! ([`architect_spaces::guest::SHARE_SHELL`]) and version, so an app of
//! another release says it is out of date, and a line that shows itself
//! after a while unless the app removed it (a bundle too old to know share
//! links).

use std::path::Path;

use architect::http::axum::body::Body;
use architect::http::axum::http::{HeaderValue, StatusCode, header};
use architect::http::axum::response::Response;
use architect_spaces::guest::{NODE_VERSION_META, SHARE_SHELL, SHARE_SHELL_META, SHARE_STALL_ID};

/// Whether the web app is served beside this node, on its origin
/// (`ARCHITECT_WEB_APP`, `Config::web_app`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebApp {
    /// None: a link's page says it needs the web app.
    #[default]
    Absent,
    /// The same release's web bundle at `/app/` on this origin.
    Beside,
}

impl WebApp {
    /// What `ARCHITECT_WEB_APP` says: unset, empty or `off` is
    /// [`Absent`](Self::Absent); `beside` is [`Beside`](Self::Beside) (a
    /// proxy serves the bundle at `/app/`, as the chart's web front end
    /// does); anything else is the folder of a web bundle a proxy serves
    /// beside the node, checked now: [`Beside`](Self::Beside) when it holds
    /// the app (`app/index.html`, `app/start.js`), else
    /// [`Absent`](Self::Absent) and the reason.
    #[must_use]
    pub fn from_setting(setting: Option<&str>) -> (Self, Option<String>) {
        match setting.map(str::trim) {
            None | Some("" | "off") => (Self::Absent, None),
            Some("beside") => (Self::Beside, None),
            Some(folder) => {
                let dir = Path::new(folder);
                let has = |f: &str| dir.join(f).is_file();
                if has("app/index.html") && has("app/start.js") {
                    (Self::Beside, None)
                } else {
                    (
                        Self::Absent,
                        Some(format!(
                            "ARCHITECT_WEB_APP: {} holds no web bundle (app/index.html, app/start.js): share links say they need the web app",
                            dir.display()
                        )),
                    )
                }
            }
        }
    }
}

/// This node's version, as its share pages carry it: the crate's, with
/// the build's revision when it has one (`ARCHITECT_BUILD_REV`).
#[must_use]
pub fn node_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    option_env!("ARCHITECT_BUILD_REV")
        .filter(|r| !r.is_empty())
        .map_or_else(
            || version.to_owned(),
            |rev| format!("{version}+{}", rev.get(..12).unwrap_or(rev)),
        )
}

/// The web app's page, as the bundle serves it at `/app/`.
const APP_PAGE: &str = include_str!("../../../web/app/index.html");

/// Where the page's title is.
const TITLE: &str = "<title>Architect</title>";
/// Where the app mounts, with what it shows while it loads.
const MOUNT: &str = "<main id=\"app\"><p class=\"muted\">Loading…</p></main>";

/// What a link's page says of the link, in its `<head>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The link's title; None when the caller may not see it yet (a
    /// signed-in link, not signed in).
    pub title: Option<String>,
    /// Whether the link is public: listed by crawlers.
    pub public: bool,
}

/// The app's page for a link, with [`Head`] in it.
///
/// # Panics
///
/// Never at run time: a test holds that the app's page still has the
/// title and mount this rewrites.
#[must_use]
pub fn page(head: &Head) -> String {
    let title = head.title.as_deref().filter(|t| !t.trim().is_empty());
    let shown = title.map_or_else(|| "Shared link".to_owned(), |t| escape(t));
    let description = title.map_or_else(
        || "A link shared with Architect.".to_owned(),
        |t| escape(&format!("{t}, shared with Architect.")),
    );
    let robots = if head.public {
        "index, nofollow"
    } else {
        "noindex, nofollow"
    };
    let version = escape(&node_version());
    let tags = format!(
        "<title>{shown} · Architect</title>\n  \
         <meta name=\"{SHARE_SHELL_META}\" content=\"{SHARE_SHELL}\">\n  \
         <meta name=\"{NODE_VERSION_META}\" content=\"{version}\">\n  \
         <meta name=\"description\" content=\"{description}\">\n  \
         <meta name=\"robots\" content=\"{robots}\">\n  \
         <meta name=\"referrer\" content=\"no-referrer\">\n  \
         <meta property=\"og:type\" content=\"website\">\n  \
         <meta property=\"og:site_name\" content=\"Architect\">\n  \
         <meta property=\"og:title\" content=\"{shown}\">\n  \
         <meta property=\"og:description\" content=\"{description}\">"
    );
    // The line a web app too old to know share links (or none at all)
    // leaves on screen: hidden for a while by an animation (no script: the
    // policy allows none inline), then shown; the app removes it once it
    // shows the link.
    let mount = format!(
        "<main id=\"app\"><p class=\"muted\">Opening the shared link…</p>\
         <noscript><p>This shared link opens in Architect, which needs JavaScript.</p></noscript></main>\n  \
         <style>#{SHARE_STALL_ID}{{visibility:hidden;animation:{SHARE_STALL_ID} 0s 15s forwards;margin:1rem;max-width:32rem}}\
         @keyframes {SHARE_STALL_ID}{{to{{visibility:visible}}}}</style>\n  \
         <p id=\"{SHARE_STALL_ID}\" role=\"status\">Still opening? This server's web app may be older than its node, \
         or not served here: the link cannot open until both are the same release.</p>"
    );
    APP_PAGE
        .replacen(TITLE, &tags, 1)
        .replacen(MOUNT, &mount, 1)
}

/// The web app's Content-Security-Policy (`scripts/web-routes.md`), with
/// `issuer` (an origin) as the one other place it may connect to.
#[must_use]
pub fn policy(issuer: Option<&str>) -> String {
    let connect = issuer
        .filter(|i| i.starts_with("https://") || i.starts_with("http://"))
        .filter(|i| !i.contains([';', ' ', ',', '\'', '"', '\r', '\n']))
        .map_or_else(|| "'self'".to_owned(), |i| format!("'self' {i}"));
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; media-src 'self'; connect-src {connect}; base-uri 'none'; form-action 'self'; object-src 'none'; frame-src 'self'; frame-ancestors 'none'"
    )
}

/// What a link's page says on a node with no web app beside it: that the
/// link opens in the web app, which this server does not serve. Nothing of
/// the link, the same for every link.
pub const NO_WEB_APP: &str = "This shared link opens in Architect's web app, which this server does not serve. Open it in the Architect app, or ask whoever runs this server to serve the web app beside it.";

/// The page as an answer with `status`.
///
/// HTML, the app's policy, never stored by anybody else (the link's own
/// headers go on in `private`). With no web app beside the node, a plain
/// `503` saying so ([`NO_WEB_APP`]) whatever `status` would have been.
#[must_use]
pub fn answer(status: StatusCode, head: &Head, issuer: Option<&str>, web: WebApp) -> Response {
    if web == WebApp::Absent {
        return no_web_app();
    }
    let mut response = Response::new(Body::from(page(head)));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    if let Ok(v) = HeaderValue::from_str(&policy(issuer)) {
        headers.insert(header::CONTENT_SECURITY_POLICY, v);
    } else {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'"),
        );
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// [`NO_WEB_APP`] as a `503`: plain text, stored by nobody, sniffed by
/// nobody.
fn no_web_app() -> Response {
    let mut response = Response::new(Body::from(NO_WEB_APP));
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_page_still_has_what_the_node_rewrites() {
        assert!(APP_PAGE.contains(TITLE), "apps/web/app/index.html: {TITLE}");
        assert!(APP_PAGE.contains(MOUNT), "apps/web/app/index.html: {MOUNT}");
        assert!(APP_PAGE.contains("<script type=\"module\" src=\"/app/start.js\"></script>"));
    }

    #[test]
    fn a_links_page_is_the_app_with_a_head_and_nothing_else() {
        let html = page(&Head {
            title: Some("Merry & <Bright>".to_owned()),
            public: false,
        });
        assert!(html.contains("<title>Merry &amp; &lt;Bright&gt; · Architect</title>"));
        assert!(html.contains("og:title\" content=\"Merry &amp; &lt;Bright&gt;\""));
        assert!(html.contains("content=\"noindex, nofollow\""));
        assert!(html.contains("<noscript>"));
        assert!(html.contains("/app/start.js"));
        // The contract and the node's version, for the app to check.
        assert!(html.contains(&format!(
            "<meta name=\"architect-share-shell\" content=\"{SHARE_SHELL}\">"
        )));
        assert!(html.contains(&format!(
            "<meta name=\"architect-node-version\" content=\"{}\">",
            node_version()
        )));
        // The line a too-old app leaves showing, hidden until then.
        assert!(html.contains("<p id=\"share-stall\""));
        assert!(html.contains("animation:share-stall 0s 15s forwards"));
        // No inline script: the app's policy would refuse it.
        assert_eq!(html.matches("<script").count(), 1, "{html}");
        let hidden = page(&Head {
            title: None,
            public: true,
        });
        assert!(hidden.contains("<title>Shared link · Architect</title>"));
        assert!(hidden.contains("content=\"index, nofollow\""));
    }

    #[test]
    fn the_policy_is_the_web_apps_own() {
        // `apps/web/sws.toml` sends it for `/app/` with production's issuer.
        let sws = include_str!("../../../web/sws.toml");
        let want = policy(Some("https://auth.fasttrackstudio.app"));
        assert!(
            sws.contains(&format!("Content-Security-Policy = \"{want}\"")),
            "the node's copy of the app's policy drifted from apps/web/sws.toml: {want}"
        );
        assert!(policy(None).contains("connect-src 'self';"));
        assert!(policy(Some("https://evil; script-src *")).contains("connect-src 'self';"));
    }

    #[test]
    fn a_node_with_no_web_app_says_so_and_nothing_of_the_link() {
        let head = Head {
            title: Some("Merry and Bright".to_owned()),
            public: true,
        };
        for status in [
            StatusCode::OK,
            StatusCode::UNAUTHORIZED,
            StatusCode::NOT_FOUND,
        ] {
            let r = answer(status, &head, None, WebApp::Absent);
            assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
            assert!(
                r.headers()[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with("text/plain")
            );
        }
        assert!(!NO_WEB_APP.contains("Merry"));
        let r = answer(StatusCode::OK, &head, None, WebApp::Beside);
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[test]
    fn the_web_app_setting_is_beside_a_bundle_or_absent() {
        assert_eq!(WebApp::from_setting(None), (WebApp::Absent, None));
        assert_eq!(WebApp::from_setting(Some(" off ")), (WebApp::Absent, None));
        assert_eq!(WebApp::from_setting(Some("beside")), (WebApp::Beside, None));
        let dir = tempfile::tempdir().unwrap();
        let (web, why) = WebApp::from_setting(dir.path().to_str());
        assert_eq!(web, WebApp::Absent);
        assert!(why.unwrap().contains("holds no web bundle"));
        std::fs::create_dir_all(dir.path().join("app")).unwrap();
        std::fs::write(dir.path().join("app/index.html"), "<!doctype html>").unwrap();
        std::fs::write(dir.path().join("app/start.js"), "").unwrap();
        assert_eq!(
            WebApp::from_setting(dir.path().to_str()),
            (WebApp::Beside, None)
        );
    }
}
