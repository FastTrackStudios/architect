//! `architect::http::ranged` end to end: an axum router serves a file, an
//! in-memory body and a caller-read streamed body, and each answers
//! `Range` with `206` + `Content-Range`, `416` when nothing is selected,
//! and the whole body without a range.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use architect::http::axum::body::{Body, Bytes, to_bytes};
use architect::http::axum::http::{HeaderMap, Request, StatusCode, header};
use architect::http::ranged::{reader_body, serve_body, serve_bytes, serve_file};
use architect::http::{Response, Router, get};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
use tower::ServiceExt as _;

const DATA: &[u8] = b"0123456789";

/// A temp file holding `DATA`, removed on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "architect-http-ranged-{tag}-{}.bin",
            std::process::id()
        ));
        std::fs::write(&path, DATA).unwrap();
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn app(file: PathBuf) -> Router {
    Router::new()
        .route(
            "/file",
            get(move |headers: HeaderMap| {
                let file = file.clone();
                async move { serve_file(&headers, &file, "audio/wav").await.unwrap() }
            }),
        )
        .route(
            "/bytes",
            get(|headers: HeaderMap| async move {
                serve_bytes(&headers, Bytes::from_static(DATA), "text/plain")
            }),
        )
        .route(
            "/stream",
            get(|headers: HeaderMap| async move {
                let total = u64::try_from(DATA.len()).unwrap();
                serve_body(&headers, total, "text/plain", |offset, len| async move {
                    let mut cursor = std::io::Cursor::new(DATA);
                    cursor.seek(std::io::SeekFrom::Start(offset)).await?;
                    Ok::<_, std::io::Error>(reader_body(cursor.take(len)))
                })
                .await
                .unwrap()
            }),
        )
}

async fn fetch(app: &Router, path: &str, range: Option<&str>) -> (Response, Bytes) {
    let mut req = Request::get(path);
    if let Some(range) = range {
        req = req.header(header::RANGE, range);
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = to_bytes(body, usize::MAX).await.unwrap();
    (Response::from_parts(parts, Body::empty()), bytes)
}

fn header_of(resp: &Response, name: header::HeaderName) -> Option<&str> {
    resp.headers().get(name).map(|v| v.to_str().unwrap())
}

#[tokio::test]
async fn file_range_0_1_is_206_with_content_range() {
    let tmp = TempFile::new("first-two");
    let app = app(tmp.0.clone());
    let (resp, body) = fetch(&app, "/file", Some("bytes=0-1")).await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        header_of(&resp, header::CONTENT_RANGE),
        Some("bytes 0-1/10")
    );
    assert_eq!(header_of(&resp, header::CONTENT_LENGTH), Some("2"));
    assert_eq!(header_of(&resp, header::ACCEPT_RANGES), Some("bytes"));
    assert_eq!(header_of(&resp, header::CONTENT_TYPE), Some("audio/wav"));
    assert_eq!(&body[..], b"01");
}

#[tokio::test]
async fn file_without_range_is_whole_200() {
    let tmp = TempFile::new("whole");
    let app = app(tmp.0.clone());
    let (resp, body) = fetch(&app, "/file", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(header_of(&resp, header::CONTENT_RANGE), None);
    assert_eq!(header_of(&resp, header::CONTENT_LENGTH), Some("10"));
    assert_eq!(header_of(&resp, header::ACCEPT_RANGES), Some("bytes"));
    assert_eq!(&body[..], DATA);
}

#[tokio::test]
async fn file_suffix_and_open_ranges() {
    let tmp = TempFile::new("suffix");
    let app = app(tmp.0.clone());
    let (resp, body) = fetch(&app, "/file", Some("bytes=-3")).await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        header_of(&resp, header::CONTENT_RANGE),
        Some("bytes 7-9/10")
    );
    assert_eq!(&body[..], b"789");

    let (resp, body) = fetch(&app, "/file", Some("bytes=4-")).await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        header_of(&resp, header::CONTENT_RANGE),
        Some("bytes 4-9/10")
    );
    assert_eq!(&body[..], b"456789");
}

#[tokio::test]
async fn unsatisfiable_is_416_on_every_helper() {
    let tmp = TempFile::new("unsat");
    let app = app(tmp.0.clone());
    for path in ["/file", "/bytes", "/stream"] {
        let (resp, body) = fetch(&app, path, Some("bytes=10-")).await;
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE, "{path}");
        assert_eq!(
            header_of(&resp, header::CONTENT_RANGE),
            Some("bytes */10"),
            "{path}"
        );
        assert!(body.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn malformed_range_is_ignored_on_every_helper() {
    let tmp = TempFile::new("malformed");
    let app = app(tmp.0.clone());
    for path in ["/file", "/bytes", "/stream"] {
        let (resp, body) = fetch(&app, path, Some("bytes=0-1,4-5")).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        assert_eq!(&body[..], DATA, "{path}");
    }
}

#[tokio::test]
async fn bytes_and_stream_slice_the_same_window() {
    let tmp = TempFile::new("same");
    let app = app(tmp.0.clone());
    for path in ["/bytes", "/stream"] {
        let (resp, body) = fetch(&app, path, Some("bytes=2-5")).await;
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT, "{path}");
        assert_eq!(
            header_of(&resp, header::CONTENT_RANGE),
            Some("bytes 2-5/10"),
            "{path}"
        );
        assert_eq!(
            header_of(&resp, header::CONTENT_LENGTH),
            Some("4"),
            "{path}"
        );
        assert_eq!(&body[..], b"2345", "{path}");
    }
}

#[tokio::test]
async fn missing_file_is_not_found_error() {
    let headers = HeaderMap::new();
    let err = serve_file(
        &headers,
        std::path::Path::new("/nonexistent/architect-ranged"),
        "x/y",
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}
