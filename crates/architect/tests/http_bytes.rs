//! HTTP-only service methods: a ranged byte download (`ByteBody`) with a
//! path parameter, a `{*path}` wildcard reading the request head, and a
//! streamed upload (`BodyStream`) past axum's 2 MiB default under
//! `limit = "1GiB"`, all under a `{slug}` service prefix, mounted through
//! `provide_http` and driven over a real socket.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::significant_drop_tightening,
    clippy::derive_partial_eq_without_eq
)]

use architect::Layer as _;

mod media {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use architect::http::axum::body::{Body, Bytes};
    use architect::http::{BodyStream, ByteBody, RequestHead};

    #[architect::error]
    pub enum MediaError {
        #[error("no such blob")]
        NotFound,
        #[error("sign in first")]
        Unauthenticated,
        #[error("too large: {0}")]
        #[architect(http_status = 413)]
        TooLarge(String),
    }

    #[architect::service(path = "org/{slug}")]
    pub trait Media {
        /// A stored blob, answering `Range`.
        #[http(get, path = "blobs/{hash}")]
        async fn blob(&self, slug: String, hash: String) -> Result<ByteBody, MediaError>;
        /// A file under the media root, for a signed-in caller.
        #[http(get, path = "media/{*path}")]
        async fn media(
            &self,
            slug: String,
            path: String,
            head: RequestHead,
        ) -> Result<ByteBody, MediaError>;
        /// Store a blob; its id comes back.
        #[http(put, path = "blobs/upload", limit = "1GiB")]
        async fn upload(&self, slug: String, body: BodyStream) -> Result<String, MediaError>;
        /// The same upload under the default 2 MiB cap.
        #[http(put, path = "notes/upload")]
        async fn note_upload(&self, slug: String, body: BodyStream) -> Result<u64, MediaError>;
        /// JSON still works on an HTTP-only service.
        #[http(get, path = "blobs")]
        async fn list(&self, slug: String) -> Result<Vec<String>, MediaError>;
    }

    #[derive(Clone, Default)]
    pub struct Backend {
        blobs: Arc<Mutex<BTreeMap<(String, String), Bytes>>>,
    }

    async fn drain(mut body: BodyStream) -> Result<Vec<u8>, MediaError> {
        let mut out = Vec::new();
        while let Some(chunk) = body.chunk().await {
            let chunk = chunk.map_err(|e| match e.kind() {
                std::io::ErrorKind::FileTooLarge => MediaError::TooLarge(e.to_string()),
                _ => MediaError::NotFound,
            })?;
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    impl Media for Backend {
        async fn blob(&self, slug: String, hash: String) -> Result<ByteBody, MediaError> {
            let bytes = self
                .blobs
                .lock()
                .unwrap()
                .get(&(slug, hash))
                .cloned()
                .ok_or(MediaError::NotFound)?;
            Ok(ByteBody::from_bytes(bytes, "application/octet-stream"))
        }

        async fn media(
            &self,
            slug: String,
            path: String,
            head: RequestHead,
        ) -> Result<ByteBody, MediaError> {
            if head.bearer() != Some("t-cody") {
                return Err(MediaError::Unauthenticated);
            }
            let text = format!("{slug}:{path}");
            let total = text.len() as u64;
            // A windowed reader: only the bytes the range selects.
            Ok(ByteBody::new(
                total,
                "text/plain",
                move |offset, len| async move {
                    let start = offset as usize;
                    let end = start + len as usize;
                    Ok(Body::from(text.as_bytes()[start..end].to_vec()))
                },
            ))
        }

        async fn upload(&self, slug: String, body: BodyStream) -> Result<String, MediaError> {
            let bytes = drain(body).await?;
            let id = format!("b{}", bytes.len());
            self.blobs
                .lock()
                .unwrap()
                .insert((slug, id.clone()), Bytes::from(bytes));
            Ok(id)
        }

        async fn note_upload(&self, _slug: String, body: BodyStream) -> Result<u64, MediaError> {
            Ok(drain(body).await?.len() as u64)
        }

        async fn list(&self, slug: String) -> Result<Vec<String>, MediaError> {
            Ok(self
                .blobs
                .lock()
                .unwrap()
                .keys()
                .filter(|(s, _)| *s == slug)
                .map(|(_, id)| id.clone())
                .collect())
        }
    }
}

async fn serve(app: architect::http::axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// 3 MiB of a repeating pattern: over axum's default cap.
fn payload() -> Vec<u8> {
    (0..3 << 20).map(|i: u32| (i % 251) as u8).collect()
}

#[tokio::test]
async fn an_upload_streams_past_the_default_cap_and_downloads_by_range() {
    let backend = media::Backend::default();
    let base = serve(architect::layers![media::Service].provide_http(&backend)).await;
    let http = reqwest::Client::new();
    let data = payload();

    // Streamed upload, 3 MiB, under `limit = "1GiB"`.
    let response = http
        .put(format!("{base}/org/acme/blobs/upload"))
        .body(data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let id: String = response.text().await.unwrap();
    assert_eq!(id, format!(r#""b{}""#, data.len()));
    let id = id.trim_matches('"').to_owned();

    // The whole blob: 200, Accept-Ranges.
    let response = http
        .get(format!("{base}/org/acme/blobs/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    assert_eq!(response.bytes().await.unwrap().as_ref(), data.as_slice());

    // A window: 206 with Content-Range, exactly those bytes.
    let response = http
        .get(format!("{base}/org/acme/blobs/{id}"))
        .header("range", "bytes=1000-1999")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.headers()["content-range"],
        format!("bytes 1000-1999/{}", data.len()).as_str()
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), &data[1000..2000]);

    // Past the end: 416.
    let response = http
        .get(format!("{base}/org/acme/blobs/{id}"))
        .header("range", format!("bytes={}-", data.len()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 416);

    // The prefix parameter scopes the store; the typed error is a 404.
    let response = http
        .get(format!("{base}/org/other/blobs/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    // A JSON method on the same service, through the typed client.
    let client = media::MediaHttpClient::at(base);
    assert_eq!(client.list("acme".into()).await.unwrap(), vec![id]);
}

#[tokio::test]
async fn a_body_over_the_route_cap_is_refused() {
    let base = serve(media::http::router(media::Backend::default())).await;
    let http = reqwest::Client::new();

    // A declared length over 2 MiB: 413 before the method runs.
    let response = http
        .put(format!("{base}/org/acme/notes/upload"))
        .body(payload())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);

    // Chunked, no length: the stream fails at the cap and the method says so.
    let chunks = payload()
        .chunks(64 << 10)
        .map(|c| Ok::<_, std::io::Error>(c.to_vec()))
        .collect::<Vec<_>>();
    let response = http
        .put(format!("{base}/org/acme/notes/upload"))
        .body(reqwest::Body::wrap_stream(futures::stream::iter(chunks)))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert!(response.text().await.unwrap().contains("too_large"));

    // Under the cap it goes through.
    let response = http
        .put(format!("{base}/org/acme/notes/upload"))
        .body(vec![7_u8; 1000])
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "1000");
}

#[tokio::test]
async fn a_wildcard_route_reads_the_request_head() {
    let base = serve(media::http::router(media::Backend::default())).await;
    let http = reqwest::Client::new();
    let url = format!("{base}/org/acme/media/docs/a%20b/readme.md");

    let response = http.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 401);
    assert!(response.text().await.unwrap().contains("unauthenticated"));

    let response = http
        .get(&url)
        .bearer_auth("t-cody")
        .header("range", "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "text/plain");
    assert_eq!(response.text().await.unwrap(), "acme");

    let response = http.get(&url).bearer_auth("t-cody").send().await.unwrap();
    assert_eq!(response.text().await.unwrap(), "acme:docs/a b/readme.md");
}
