//! `EngineHost::site`: a built static site over the host's HTTP face
//! (#364), at the root or nested, with its cache headers and its own
//! `404.html`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use architect::LayerRouter;
use architect::host::{CACHE_FILE, CACHE_IMMUTABLE, CACHE_PAGE, EngineHost};
use architect::http::axum::Router;
use architect::http::axum::body::{Body, to_bytes};
use architect::http::axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

/// A site as `architect site build` writes one.
fn site() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("guide/ui")).unwrap();
    std::fs::create_dir_all(root.join("_site")).unwrap();
    std::fs::write(root.join("index.html"), "<p>home</p>").unwrap();
    std::fs::write(root.join("guide/ui/index.html"), "<p>ui</p>").unwrap();
    std::fs::write(root.join("404.html"), "<p>lost</p>").unwrap();
    std::fs::write(root.join("_site/site.0a1b2c3d.css"), "body{}").unwrap();
    std::fs::write(root.join("logo.svg"), "<svg/>").unwrap();
    dir
}

/// Status, `Cache-Control` and body of `GET path`.
async fn get(app: Router, path: &str) -> (StatusCode, String, String) {
    let res = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let cache = res
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, cache, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn a_site_at_the_root_serves_pages_assets_and_its_404() {
    let dir = site();
    let app = EngineHost::ungated(LayerRouter::new(), "127.0.0.1:0")
        .site("/", dir.path())
        .into_app();

    let page = get(app.clone(), "/guide/ui/").await;
    assert_eq!(
        page,
        (
            StatusCode::OK,
            CACHE_PAGE.to_owned(),
            "<p>ui</p>".to_owned()
        )
    );
    let home = get(app.clone(), "/").await;
    assert_eq!((home.0, home.1.as_str()), (StatusCode::OK, CACHE_PAGE));

    let css = get(app.clone(), "/_site/site.0a1b2c3d.css").await;
    assert_eq!((css.0, css.1.as_str()), (StatusCode::OK, CACHE_IMMUTABLE));
    let logo = get(app.clone(), "/logo.svg").await;
    assert_eq!((logo.0, logo.1.as_str()), (StatusCode::OK, CACHE_FILE));

    // Not a SPA: a path the site lacks is its 404 page, with a 404.
    let lost = get(app.clone(), "/nope/").await;
    assert_eq!(
        (lost.0, lost.2.as_str()),
        (StatusCode::NOT_FOUND, "<p>lost</p>")
    );

    // The host's own routes still win.
    let health = get(app, "/health").await;
    assert_eq!((health.0, health.2.as_str()), (StatusCode::OK, "ok"));
}

#[tokio::test]
async fn a_nested_site_answers_under_its_mount_only() {
    let dir = site();
    let app = EngineHost::ungated(LayerRouter::new(), "127.0.0.1:0")
        .site("/docs/", dir.path())
        .into_app();

    let page = get(app.clone(), "/docs/guide/ui/").await;
    assert_eq!((page.0, page.2.as_str()), (StatusCode::OK, "<p>ui</p>"));
    let css = get(app.clone(), "/docs/_site/site.0a1b2c3d.css").await;
    assert_eq!(css.1, CACHE_IMMUTABLE);
    let outside = get(app, "/guide/ui/").await;
    assert_eq!(outside.0, StatusCode::NOT_FOUND);
}

/// `BUILT_PAGE` marks a folder's page the builder wrote, and nothing else:
/// not a file of the site, not an `.html` one, not the 404 page.
#[tokio::test]
async fn a_built_page_is_marked_and_nothing_else_is() {
    use architect::host::BUILT_PAGE;
    let dir = site();
    std::fs::write(dir.path().join("raw.html"), "<p>raw</p>").unwrap();
    let app = EngineHost::ungated(LayerRouter::new(), "127.0.0.1:0")
        .site("/", dir.path())
        .into_app();
    let marked = |path: &'static str| {
        let app = app.clone();
        async move {
            let res = app
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            (
                res.status(),
                res.headers()
                    .get(BUILT_PAGE.0)
                    .map(|v| v.to_str().unwrap().to_owned()),
            )
        }
    };
    assert_eq!(
        marked("/guide/ui/").await,
        (StatusCode::OK, Some(BUILT_PAGE.1.to_owned()))
    );
    assert_eq!(marked("/").await.1.as_deref(), Some(BUILT_PAGE.1));
    assert_eq!(marked("/raw.html").await, (StatusCode::OK, None));
    assert_eq!(marked("/logo.svg").await, (StatusCode::OK, None));
    assert_eq!(marked("/nope/").await, (StatusCode::NOT_FOUND, None));
}
