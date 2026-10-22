//! `GET /version.json` on `EngineHost`: it answers the binary's
//! `BuildInfo` as JSON, the SPA fallback never answers it, and an unknown
//! path still falls through to the bundle.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use architect::LayerRouter;
use architect::host::{BuildInfo, EngineHost, WebBundle};
use architect::http::axum::Router;
use architect::http::axum::body::{Body, to_bytes};
use architect::http::axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

const SPA: &str = "<!doctype html><title>spa</title>";

/// A SPA bundle dir with only `index.html`, unique per test.
fn spa_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "architect-version-json-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), SPA).unwrap();
    dir
}

async fn get(app: Router, path: &str) -> (StatusCode, Option<String>, Option<String>, String) {
    let res = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let header_str = |name| {
        headers
            .get(name)
            .map(|v: &header::HeaderValue| v.to_str().unwrap().to_owned())
    };
    let content_type = header_str(header::CONTENT_TYPE);
    let cache = header_str(header::CACHE_CONTROL);
    let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (
        status,
        content_type,
        cache,
        String::from_utf8(body.to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn version_json_wins_over_the_spa_and_unknown_paths_still_fall_back() {
    let dir = spa_dir("spa");
    let app = || {
        EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
            .web(Some(WebBundle::Dir(dir.clone())))
            .build_info(BuildInfo::new(
                Some("1.2.3"),
                Some("0123abcd"),
                Some("2026-10-22T08:00:00Z"),
            ))
            .into_app()
    };

    let (status, content_type, cache, body) = get(app(), "/version.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    assert_eq!(cache.as_deref(), Some("no-store"));
    assert_eq!(
        body,
        r#"{"rev":"0123abcd","version":"1.2.3","built_at":"2026-10-22T08:00:00Z"}"#
    );

    let (status, _, _, body) = get(app(), "/no/such/page").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, SPA, "an unknown path still gets the SPA index");

    let (status, _, _, body) = get(app(), "/health").await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "ok"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn unset_fields_are_null_and_empty_env_counts_as_unset() {
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .build_info(BuildInfo::new(Some("0.1.0"), Some(""), None))
        .into_app();
    let (status, _, _, body) = get(app, "/version.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"rev":null,"version":"0.1.0","built_at":null}"#);

    // No `build_info` at all: the route still answers, every field null,
    // and a headless host (no bundle) has no fallback to answer for it.
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0").into_app();
    let (status, _, _, body) = get(app.clone(), "/version.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"rev":null,"version":null,"built_at":null}"#);
    let (status, _, _, _) = get(app, "/no/such/page").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn build_info_macro_reads_the_calling_crate() {
    let info = architect::build_info!();
    assert_eq!(info.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(
        info.rev.as_deref(),
        option_env!("ARCHITECT_BUILD_REV").filter(|s| !s.is_empty())
    );
    assert_eq!(
        info.built_at.as_deref(),
        option_env!("ARCHITECT_BUILT_AT").filter(|s| !s.is_empty())
    );
    assert_eq!(architect::host::BUILD_REV_ENV, "ARCHITECT_BUILD_REV");
    assert_eq!(architect::host::BUILT_AT_ENV, "ARCHITECT_BUILT_AT");
}
