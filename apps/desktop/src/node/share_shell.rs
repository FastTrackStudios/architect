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

use architect::http::axum::body::Body;
use architect::http::axum::http::{HeaderValue, StatusCode, header};
use architect::http::axum::response::Response;

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
    let tags = format!(
        "<title>{shown} · Architect</title>\n  \
         <meta name=\"description\" content=\"{description}\">\n  \
         <meta name=\"robots\" content=\"{robots}\">\n  \
         <meta name=\"referrer\" content=\"no-referrer\">\n  \
         <meta property=\"og:type\" content=\"website\">\n  \
         <meta property=\"og:site_name\" content=\"Architect\">\n  \
         <meta property=\"og:title\" content=\"{shown}\">\n  \
         <meta property=\"og:description\" content=\"{description}\">"
    );
    let mount = "<main id=\"app\"><p class=\"muted\">Opening the shared link…</p>\
                 <noscript><p>This shared link opens in Architect, which needs JavaScript.</p></noscript></main>";
    APP_PAGE.replacen(TITLE, &tags, 1).replacen(MOUNT, mount, 1)
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

/// The page as an answer with `status`: HTML, the app's policy, never
/// stored by anybody else (the link's own headers go on in `private`).
#[must_use]
pub fn answer(status: StatusCode, head: &Head, issuer: Option<&str>) -> Response {
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
}
