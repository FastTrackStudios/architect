//! A server's copy of blobs over HTTP: `GET <base>/<hex>` with a `Range`
//! header, the route [`super::serve_http`] answers (or any server that
//! honours `Range` on a URL per hash).

use std::io;

use futures::StreamExt as _;
use futures::future::BoxFuture;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};

use super::stream_reader::StreamReader;
use super::{Reader, Remote, Sha256};

/// Blobs at `<base>/<hex>` over HTTP, as a [`Remote`].
#[derive(Clone, Debug)]
pub struct HttpRemote {
    base: String,
    client: reqwest::Client,
}

impl HttpRemote {
    /// Blobs at `<base>/<hex>` (`https://task.example/org/acme/blobs`).
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            client: reqwest::Client::new(),
        }
    }

    /// Send through `client` (its default headers carry a bearer, its
    /// timeouts apply).
    #[must_use]
    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    fn url(&self, hash: Sha256) -> String {
        format!("{}/{hash}", self.base)
    }
}

fn http_err(e: reqwest::Error) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, e)
}

fn status_err(status: StatusCode) -> io::Error {
    io::Error::other(format!("blob server answered {status}"))
}

/// The total in `Content-Range: bytes a-b/total` (or `bytes */total`).
fn total_of(content_range: &str) -> Option<u64> {
    content_range.rsplit_once('/')?.1.trim().parse().ok()
}

impl Remote for HttpRemote {
    fn size(&self, hash: Sha256) -> BoxFuture<'_, io::Result<Option<u64>>> {
        Box::pin(async move {
            // One byte, so the length comes back in `Content-Range`
            // without the body.
            let res = self
                .client
                .get(self.url(hash))
                .header(RANGE, "bytes=0-0")
                .send()
                .await
                .map_err(http_err)?;
            let header = |name| {
                res.headers()
                    .get(name)
                    .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
                    .map(str::to_owned)
            };
            match res.status() {
                StatusCode::NOT_FOUND => Ok(None),
                StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE => {
                    header(CONTENT_RANGE)
                        .as_deref()
                        .and_then(total_of)
                        .map(Some)
                        .ok_or_else(|| io::Error::other("blob server sent no Content-Range"))
                }
                StatusCode::OK => header(CONTENT_LENGTH)
                    .and_then(|v| v.parse().ok())
                    .map(Some)
                    .ok_or_else(|| io::Error::other("blob server sent no Content-Length")),
                other => Err(status_err(other)),
            }
        })
    }

    fn read_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        Box::pin(async move {
            let res = self
                .client
                .get(self.url(hash))
                .header(RANGE, format!("bytes={offset}-"))
                .send()
                .await
                .map_err(http_err)?;
            match res.status() {
                StatusCode::PARTIAL_CONTENT => {}
                // The whole body is only right from the start; a server
                // that ignores `Range` cannot resume.
                StatusCode::OK if offset == 0 => {}
                other => return Err(status_err(other)),
            }
            let body = res
                .bytes_stream()
                .map(|chunk| chunk.map(|b| b.to_vec()).map_err(http_err));
            let reader: Reader = Box::pin(StreamReader::new(Box::pin(body)));
            Ok(reader)
        })
    }
}
