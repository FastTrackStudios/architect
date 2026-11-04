//! HTTP-only service methods: a ranged byte download (`ByteBody`) with a
//! path parameter, a `{*path}` wildcard reading the request head, and a
//! streamed upload (`BodyStream`) past axum's 2 MiB default under
//! `limit = "1GiB"`, and query-string arguments on both a download and an
//! upload, all under a `{slug}` service prefix, mounted through
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
        /// An upload named by its query: `PUT …/files?name=…&part=…`.
        #[http(put, path = "files", limit = "1GiB")]
        async fn put_file(
            &self,
            slug: String,
            name: String,
            part: u32,
            body: BodyStream,
        ) -> Result<String, MediaError>;
        /// A stored blob behind a share password: `GET …/share/{hash}?pw=…`.
        #[http(get, path = "share/{hash}")]
        async fn shared(
            &self,
            slug: String,
            hash: String,
            pw: Option<String>,
        ) -> Result<ByteBody, MediaError>;
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

        async fn put_file(
            &self,
            slug: String,
            name: String,
            part: u32,
            body: BodyStream,
        ) -> Result<String, MediaError> {
            let bytes = drain(body).await?;
            let id = format!("{name}#{part}");
            self.blobs
                .lock()
                .unwrap()
                .insert((slug, id.clone()), Bytes::from(bytes));
            Ok(id)
        }

        async fn shared(
            &self,
            slug: String,
            hash: String,
            pw: Option<String>,
        ) -> Result<ByteBody, MediaError> {
            if pw.as_deref() != Some("open sesame") {
                return Err(MediaError::Unauthenticated);
            }
            self.blob(slug, hash).await
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

/// A `PUT` written by hand: the head, then exactly `body`, then the reply
/// read to its end (the request says `Connection: close`).
///
/// Not reqwest, for a refused body: it keeps writing the body after the
/// server has answered `413` without reading it, and a server that closes
/// a socket with unread bytes in it resets the connection, so the write
/// fails with `ECONNRESET` before the client reads the reply. On Linux's
/// loopback the 3 MiB usually fits in the socket buffers first; on macOS
/// it mostly does not. Sending only what the server reads makes its close
/// a plain end of stream on both.
async fn raw_put(base: &str, path: &str, headers: &str, body: &[u8]) -> (u16, String) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let addr = base.trim_start_matches("http://");
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!("PUT {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n{headers}\r\n");
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(body).await.unwrap();
    let mut reply = Vec::new();
    socket.read_to_end(&mut reply).await.unwrap();
    let reply = String::from_utf8(reply).unwrap();
    let status = reply.split(' ').nth(1).unwrap().parse().unwrap();
    (status, reply)
}

#[tokio::test]
async fn a_body_over_the_route_cap_is_refused() {
    let base = serve(media::http::router(media::Backend::default())).await;
    let http = reqwest::Client::new();
    let cap = 2 << 20;

    // A declared length over 2 MiB: 413 from the head alone, before the
    // method runs; none of the body is sent.
    let (status, _) = raw_put(
        &base,
        "/org/acme/notes/upload",
        &format!("content-length: {}\r\n", payload().len()),
        b"",
    )
    .await;
    assert_eq!(status, 413);

    // Chunked, no length: the stream fails at the cap and the method says
    // so. 2 MiB in 64 KiB chunks, then a chunk of one byte past the cap,
    // and nothing after it: the server reads all of it.
    let mut chunked = Vec::new();
    for chunk in payload()[..cap].chunks(64 << 10) {
        chunked.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        chunked.extend_from_slice(chunk);
        chunked.extend_from_slice(b"\r\n");
    }
    chunked.extend_from_slice(b"1\r\n\x07");
    let (status, reply) = raw_put(
        &base,
        "/org/acme/notes/upload",
        "transfer-encoding: chunked\r\n",
        &chunked,
    )
    .await;
    assert_eq!(status, 413);
    assert!(reply.contains("too_large"), "{reply}");

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

#[tokio::test]
async fn query_arguments_ride_an_upload_and_a_ranged_download() {
    let backend = media::Backend::default();
    let base = serve(architect::layers![media::Service].provide_http(&backend)).await;
    let http = reqwest::Client::new();
    let data = payload();

    // A streamed upload whose other arguments are the query: 3 MiB under
    // the route's 1 GiB cap, the name decoded (`%20`), the part parsed.
    let response = http
        .put(format!("{base}/org/acme/files?name=a%20b.txt&part=2"))
        .body(data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), r#""a b.txt#2""#);

    // A query argument that is missing or does not parse is a 400 with
    // the usual envelope, before the method runs.
    for query in ["name=x", "name=x&part=two"] {
        let response = http
            .put(format!("{base}/org/acme/files?{query}"))
            .body(vec![1_u8; 10])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{query}");
        assert!(
            response.text().await.unwrap().contains("invalid_query"),
            "{query}"
        );
    }

    // An `Option` argument: absent reads as `None`, so the share refuses.
    let url = format!("{base}/org/acme/share/{}", "a%20b.txt%232");
    let response = http.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 401);
    // With it (`+` is a space), the download still answers `Range`.
    let response = http
        .get(format!("{url}?pw=open+sesame"))
        .header("range", "bytes=10-19")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.bytes().await.unwrap().as_ref(), &data[10..20]);
}
