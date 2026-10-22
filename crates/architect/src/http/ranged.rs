//! `architect::http::ranged` — answer a single `Range: bytes=…` request
//! with `206 Partial Content`.
//!
//! An `<audio>` or `<video>` element seeks by asking for a byte range, and
//! a resumable download does the same. A route that hands back the whole
//! body every time makes the browser buffer a whole stem before it can
//! seek. Task's attachment download carried its own parser and response
//! assembly for this; every app that serves bytes over HTTP needs the same
//! dozen rules, so they live here once.
//!
//! - [`parse_range`] is pure and builds everywhere (wasm included): a
//!   `Range` header value plus the resource length in, a [`ByteRange`] out.
//! - Under the `http` feature: [`range_of`] reads the header off a request,
//!   [`respond`] assembles the `200`/`206`/`416` response around a body,
//!   [`serve_bytes`] slices an in-memory body, and [`serve_body`] asks the
//!   caller for only the bytes the range selects (a blob store, an object
//!   store, a database page).
//! - Natively, also [`serve_file`] (a file on disk, streamed) and
//!   [`reader_body`] (any `tokio::io::AsyncRead` as a streamed body).
//!
//! The rules, from RFC 9110 §14:
//!
//! | `Range` | answer |
//! |---|---|
//! | absent, or not something this module understands | `200`, whole body |
//! | `bytes=a-b`, `bytes=a-`, `bytes=-n` that selects bytes | `206` + `Content-Range: bytes a-b/total` |
//! | a well-formed `bytes=` spec that selects nothing | `416` + `Content-Range: bytes */total` |
//!
//! A server may ignore `Range`, so anything malformed (another unit, junk
//! numbers, several ranges) is served whole rather than refused. Multiple
//! ranges (`multipart/byteranges`) are deliberately not supported: media
//! elements never send them.
//!
//! What this does not do: conditional requests (`If-Range`,
//! `If-None-Match`, `ETag`, `Last-Modified`), `HEAD`, or a guessed content
//! type. For a file on disk with none of your own logic in front of it,
//! `tower_http::services::ServeFile` / `ServeDir` already do all of that;
//! see the "Byte ranges" section of `docs/content/architecture/http.md`.

/// Chunk size for streamed bodies ([`reader_body`], [`serve_file`]).
pub const CHUNK: usize = 65_536;

/// How a request's `Range` header resolves against a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    /// No `Range` header, or one malformed enough to ignore (RFC 9110 lets
    /// a server ignore `Range`): serve the whole body, `200`.
    Absent,
    /// One satisfiable `bytes=` range, resolved to the inclusive
    /// `(first, last)` byte positions: `206`.
    Satisfiable(u64, u64),
    /// A well-formed `bytes=` spec that selects nothing: `416`.
    Unsatisfiable,
}

impl ByteRange {
    /// The bytes the response body carries, as `(offset, length)` into a
    /// resource of `total` bytes; `None` for a `416`, which has no body.
    #[must_use]
    pub const fn window(self, total: u64) -> Option<(u64, u64)> {
        match self {
            Self::Absent => Some((0, total)),
            Self::Satisfiable(first, last) => {
                Some((first, last.saturating_sub(first).saturating_add(1)))
            }
            Self::Unsatisfiable => None,
        }
    }
}

/// Resolve a `Range` header value against a resource of `total` bytes.
///
/// Single range only: `bytes=a-b`, `bytes=a-` (to the end) and `bytes=-n`
/// (the last `n`). A `last` past the end is clamped to it. Anything not of
/// that shape is [`ByteRange::Absent`] (serve whole); a well-formed spec
/// that selects no byte is [`ByteRange::Unsatisfiable`].
///
/// ```
/// use architect::http::ranged::{ByteRange, parse_range};
/// assert_eq!(parse_range("bytes=0-1", 10), ByteRange::Satisfiable(0, 1));
/// assert_eq!(parse_range("bytes=-3", 10), ByteRange::Satisfiable(7, 9));
/// assert_eq!(parse_range("bytes=10-", 10), ByteRange::Unsatisfiable);
/// assert_eq!(parse_range("bytes=0-1,4-5", 10), ByteRange::Absent);
/// ```
#[must_use]
pub fn parse_range(raw: &str, total: u64) -> ByteRange {
    let Some(spec) = raw.trim().strip_prefix("bytes=") else {
        return ByteRange::Absent;
    };
    let spec = spec.trim();
    if spec.contains(',') {
        return ByteRange::Absent;
    }
    let Some((from, to)) = spec.split_once('-') else {
        return ByteRange::Absent;
    };
    let (from, to) = (from.trim(), to.trim());
    let last_byte = total.saturating_sub(1);
    let (first, last) = match (from.parse::<u64>(), to.parse::<u64>()) {
        // `bytes=a-b`
        (Ok(a), Ok(b)) => (a, b),
        // `bytes=a-`: to the end
        (Ok(a), Err(_)) if to.is_empty() => (a, last_byte),
        // `bytes=-n`: the last n bytes
        (Err(_), Ok(n)) if from.is_empty() => {
            if n == 0 || total == 0 {
                return ByteRange::Unsatisfiable;
            }
            (total.saturating_sub(n), last_byte)
        }
        _ => return ByteRange::Absent,
    };
    if first <= last && first < total {
        ByteRange::Satisfiable(first, last.min(last_byte))
    } else {
        ByteRange::Unsatisfiable
    }
}

/// The `Content-Range` value of a `206`: `bytes first-last/total`.
#[must_use]
pub fn content_range(first: u64, last: u64, total: u64) -> String {
    format!("bytes {first}-{last}/{total}")
}

/// The `Content-Range` value of a `416`: `bytes */total`.
#[must_use]
pub fn unsatisfied_range(total: u64) -> String {
    format!("bytes */{total}")
}

#[cfg(feature = "http")]
mod serve {
    use super::{ByteRange, content_range, parse_range, unsatisfied_range};
    use axum::body::{Body, Bytes};
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use axum::response::{IntoResponse, Response};
    use core::future::Future;

    /// The request's `Range` header resolved against `total` bytes; a
    /// header that is not visible ASCII counts as absent.
    #[must_use]
    pub fn range_of(headers: &HeaderMap, total: u64) -> ByteRange {
        headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map_or(ByteRange::Absent, |raw| parse_range(raw, total))
    }

    /// Wrap `body` in the response `range` calls for: `200` with the whole
    /// length, `206` with `Content-Range`, or a bodiless `416` (whatever
    /// `body` holds is dropped). `body` must already be exactly the bytes
    /// [`ByteRange::window`] selects. Every answer says
    /// `Accept-Ranges: bytes`. A `content_type` that is not a valid header
    /// value is sent as `application/octet-stream`.
    #[must_use]
    pub fn respond(range: ByteRange, total: u64, content_type: &str, body: Body) -> Response {
        let accept = (header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        let Some((_, len)) = range.window(total) else {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [accept, (header::CONTENT_RANGE, header_value(&unsatisfied_range(total)))],
            )
                .into_response();
        };
        let kind = HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
        let headers = [
            accept,
            (header::CONTENT_TYPE, kind),
            (header::CONTENT_LENGTH, HeaderValue::from(len)),
        ];
        match range {
            ByteRange::Satisfiable(first, last) => (
                StatusCode::PARTIAL_CONTENT,
                headers,
                [(header::CONTENT_RANGE, header_value(&content_range(first, last, total)))],
                body,
            )
                .into_response(),
            ByteRange::Absent | ByteRange::Unsatisfiable => {
                (StatusCode::OK, headers, body).into_response()
            }
        }
    }

    /// Serve a resource of `total` bytes whose contents the caller reads
    /// itself. `read(offset, len)` is called once, with exactly the window
    /// the request selects (`(0, total)` without a range), and not at all
    /// for a `416`; hand back a buffered or a streamed [`Body`]
    /// ([`super::reader_body`], `Body::from_stream`). Its error is returned
    /// as is, so the route decides what a failed read looks like.
    ///
    /// ```ignore
    /// let total = store.len(&hash).await?;
    /// serve_body(&headers, total, &mime, |offset, len| async move {
    ///     store.read_range(&hash, offset, len).await.map(Body::from)
    /// })
    /// .await
    /// ```
    pub async fn serve_body<F, Fut, E>(
        headers: &HeaderMap,
        total: u64,
        content_type: &str,
        read: F,
    ) -> Result<Response, E>
    where
        F: FnOnce(u64, u64) -> Fut + Send,
        Fut: Future<Output = Result<Body, E>> + Send,
    {
        let range = range_of(headers, total);
        let Some((offset, len)) = range.window(total) else {
            return Ok(respond(range, total, content_type, Body::empty()));
        };
        let body = read(offset, len).await?;
        Ok(respond(range, total, content_type, body))
    }

    /// Serve an in-memory body, sliced to the request's range. `Bytes`
    /// slices share the buffer; nothing is copied.
    #[must_use]
    pub fn serve_bytes(headers: &HeaderMap, bytes: Bytes, content_type: &str) -> Response {
        let total = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let range = range_of(headers, total);
        let body = range
            .window(total)
            .and_then(|(offset, len)| {
                let start = usize::try_from(offset).ok()?;
                let end = start.checked_add(usize::try_from(len).ok()?)?;
                (end <= bytes.len()).then(|| bytes.slice(start..end))
            })
            .unwrap_or_default();
        respond(range, total, content_type, Body::from(body))
    }

    /// A header value built from text this module formatted (ASCII digits,
    /// `bytes`, `-`, `/`, `*`), so it is always valid; the fallback is
    /// only there to keep the code panic-free.
    fn header_value(text: &str) -> HeaderValue {
        HeaderValue::from_str(text).unwrap_or_else(|_| HeaderValue::from_static("bytes */0"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod native {
        use super::serve_body;
        use crate::http::ranged::CHUNK;
        use axum::body::{Body, Bytes};
        use axum::http::HeaderMap;
        use axum::response::Response;
        use std::io::SeekFrom;
        use std::path::Path;
        use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncSeekExt as _};

        /// Stream any `AsyncRead` as a response body, [`CHUNK`] bytes
        /// at a time. A read error ends the body with that error
        /// (the connection is cut; the status line is already sent). Pair
        /// it with `AsyncReadExt::take` to stop at a range's end.
        #[must_use]
        pub fn reader_body<R>(reader: R) -> Body
        where
            R: AsyncRead + Unpin + Send + 'static,
        {
            let chunks = futures::stream::unfold(Some(reader), |state| async move {
                let mut reader = state?;
                let mut buf = vec![0_u8; CHUNK];
                match reader.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        Some((Ok(Bytes::from(buf)), Some(reader)))
                    }
                    Err(e) => Some((Err(e), None)),
                }
            });
            Body::from_stream(chunks)
        }

        /// Serve a file from disk, streamed, honouring `Range`. The length
        /// comes from the file's metadata at open time. Errors are opening,
        /// stat-ing or seeking the file (a missing file is
        /// `io::ErrorKind::NotFound`; map it to `404` in the route).
        pub async fn serve_file(
            headers: &HeaderMap,
            path: &Path,
            content_type: &str,
        ) -> std::io::Result<Response> {
            let mut file = tokio::fs::File::open(path).await?;
            let total = file.metadata().await?.len();
            serve_body(headers, total, content_type, |offset, len| async move {
                file.seek(SeekFrom::Start(offset)).await?;
                Ok(reader_body(file.take(len)))
            })
            .await
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub use native::{reader_body, serve_file};
}

#[cfg(feature = "http")]
pub use serve::{range_of, respond, serve_body, serve_bytes};
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub use serve::{reader_body, serve_file};

#[cfg(test)]
mod tests {
    use super::{ByteRange, parse_range};

    // Ported from Task's attachment routes (`range_tests`), where this
    // parser served `<audio>` seeks before it moved here.

    #[test]
    fn plain_ranges_resolve() {
        assert_eq!(parse_range("bytes=0-499", 1000), ByteRange::Satisfiable(0, 499));
        assert_eq!(parse_range("bytes=500-", 1000), ByteRange::Satisfiable(500, 999));
        assert_eq!(parse_range("bytes=-200", 1000), ByteRange::Satisfiable(800, 999));
        // A last byte past the end is clamped.
        assert_eq!(parse_range("bytes=900-2000", 1000), ByteRange::Satisfiable(900, 999));
        // A suffix longer than the resource is the whole resource.
        assert_eq!(parse_range("bytes=-5000", 1000), ByteRange::Satisfiable(0, 999));
        assert_eq!(parse_range(" bytes= 0 - 1 ", 1000), ByteRange::Satisfiable(0, 1));
    }

    #[test]
    fn unsatisfiable_ranges_416() {
        assert_eq!(parse_range("bytes=1000-", 1000), ByteRange::Unsatisfiable);
        assert_eq!(parse_range("bytes=5-2", 1000), ByteRange::Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 1000), ByteRange::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-", 0), ByteRange::Unsatisfiable);
        assert_eq!(parse_range("bytes=-1", 0), ByteRange::Unsatisfiable);
    }

    #[test]
    fn malformed_ranges_ignored() {
        assert_eq!(parse_range("chunks=0-1", 1000), ByteRange::Absent);
        assert_eq!(parse_range("bytes=a-b", 1000), ByteRange::Absent);
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), ByteRange::Absent);
        assert_eq!(parse_range("bytes=-", 1000), ByteRange::Absent);
        assert_eq!(parse_range("bytes=1", 1000), ByteRange::Absent);
        assert_eq!(parse_range("", 1000), ByteRange::Absent);
    }

    #[test]
    fn window_is_offset_and_length() {
        assert_eq!(ByteRange::Absent.window(10), Some((0, 10)));
        assert_eq!(ByteRange::Satisfiable(0, 1).window(10), Some((0, 2)));
        assert_eq!(ByteRange::Satisfiable(7, 9).window(10), Some((7, 3)));
        assert_eq!(ByteRange::Unsatisfiable.window(10), None);
    }
}
