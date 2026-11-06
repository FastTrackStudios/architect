//! A built static site served over the host's HTTP face (#364):
//! [`EngineHost::site`], and [`site_router`] for a binary that serves one
//! without a host (`architect site serve`).
//!
//! A site is a folder of finished files, as `architect-sites` writes it:
//! `index.html` per page, `404.html`, and its fingerprinted assets (CSS,
//! scripts, the search index) under `_site/`. The host only serves it,
//! and says how long each answer may be kept:
//!
//! | what                                   | `Cache-Control`                       |
//! |----------------------------------------|---------------------------------------|
//! | a page (`text/html`, or a folder path) | `no-cache` (revalidated every time)   |
//! | `_site/*` (named by content hash)      | `public, max-age=31536000, immutable` |
//! | anything else (images, attachments)    | `public, max-age=3600`                |
//!
//! A page is revalidated rather than kept because a deploy changes it
//! under the same URL; the `Last-Modified` answer makes that a `304` when
//! it has not. An asset under `_site/` never changes under its name, so a
//! browser keeps it for a year. A path that names nothing gets the site's
//! own `404.html`, with a `404`.

use std::path::PathBuf;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::services::{ServeDir, ServeFile};

use super::EngineHost;

/// Where a site keeps its fingerprinted assets: every file below it is
/// named by its content, so it is cached for good.
pub const SITE_ASSETS: &str = "_site";

/// A page: revalidated on every load.
pub const CACHE_PAGE: &str = "no-cache";
/// A fingerprinted asset: kept a year, never revalidated.
pub const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// Anything else a site ships (images, attachments): an hour.
pub const CACHE_FILE: &str = "public, max-age=3600";

/// The `Cache-Control` for a successful answer to `path` (the path within
/// the site) that came back as `content_type`.
#[must_use]
pub fn cache_policy(path: &str, content_type: Option<&str>) -> &'static str {
    let in_assets = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .is_some_and(|first| first == SITE_ASSETS);
    if in_assets {
        CACHE_IMMUTABLE
    } else if path.ends_with('/')
        || std::path::Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("html"))
        || content_type.is_some_and(|t| t.starts_with("text/html"))
    {
        CACHE_PAGE
    } else {
        CACHE_FILE
    }
}

/// The router that serves the site in `dir` from its root.
///
/// Each folder answers with its `index.html`, a path that is not there
/// with `404.html`, and every answer carries [`cache_policy`]'s header.
/// Mount it with [`EngineHost::site`], or nest it in a router of your own.
#[must_use]
pub fn site_router(dir: impl Into<PathBuf>) -> Router {
    let dir = dir.into();
    let files = ServeDir::new(&dir)
        .append_index_html_on_directories(true)
        .not_found_service(ServeFile::new(dir.join("404.html")));
    Router::new()
        .fallback_service(files)
        .layer(middleware::from_fn(cache_headers))
}

/// Puts [`cache_policy`]'s answer on every success and `304`.
async fn cache_headers(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let status = response.status();
    if status.is_success() || status == StatusCode::NOT_MODIFIED {
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok());
        let policy = cache_policy(&path, content_type);
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(policy));
    } else if status == StatusCode::NOT_FOUND {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_PAGE));
    }
    response
}

impl EngineHost {
    /// Serve the built static site in `dir` at `mount` (`/` or `/docs`):
    /// a project hosts its own docs from the server it already runs.
    ///
    /// At `/` the site is the host's fallback, in place of a
    /// [`web`](Self::web) bundle (the later call wins); anywhere else it is
    /// nested there, beside the host's other routes. `/health`, `/vox` and
    /// every plugin route still win over a page of the same path. Cache
    /// headers are [`cache_policy`]'s.
    ///
    /// ```ignore
    /// EngineHost::new(router, gate, "0.0.0.0:4040")
    ///     .site("/docs", "target/site")
    ///     .serve()
    ///     .await
    /// ```
    #[must_use]
    pub fn site(mut self, mount: &str, dir: impl Into<PathBuf>) -> Self {
        let mount = mount.trim_end_matches('/');
        if mount.is_empty() {
            self.web = None;
            self.site_root = Some(dir.into());
            return self;
        }
        let routes = Router::new().nest_service(mount, site_router(dir));
        self.extend(routes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_are_immutable_pages_revalidate_files_keep_an_hour() {
        assert_eq!(cache_policy("/_site/site.3f2a.css", None), CACHE_IMMUTABLE);
        assert_eq!(cache_policy("/guide/", None), CACHE_PAGE);
        assert_eq!(cache_policy("/guide/index.html", None), CACHE_PAGE);
        assert_eq!(
            cache_policy("/guide", Some("text/html; charset=utf-8")),
            CACHE_PAGE
        );
        assert_eq!(cache_policy("/img/logo.png", Some("image/png")), CACHE_FILE);
        // Only the top-level folder is the assets'.
        assert_eq!(cache_policy("/guide/_site/x.css", None), CACHE_FILE);
    }
}
