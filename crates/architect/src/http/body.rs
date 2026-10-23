//! The byte half of the HTTP face: what an HTTP-only service method takes
//! and returns instead of JSON.
//!
//! - [`BodyStream`]: the request body, streamed and capped at the route's
//!   `limit`, for uploads.
//! - [`RequestHead`]: the method, URI, headers and path params, for
//!   `Authorization`, a query string or anything else the arguments do not
//!   carry. The same type every `EngineHost` protocol mount's hook sees.
//! - [`ByteBody`]: a response of `total` bytes of one content type, read a
//!   window at a time. The generated handler serves it through
//!   [`ranged::serve_body`](super::ranged::serve_body), so every byte route
//!   answers `Range` with `206` / `416` without asking.
//!
//! The types exist in every build so a trait that names them compiles in a
//! wasm client too; constructing and consuming them needs the `http`
//! feature (the server side).

#[cfg(feature = "http")]
mod imp {
    use super::super::{HttpError, Response, err, plain_error};
    use axum::body::{Body, BodyDataStream, Bytes};
    use axum::extract::rejection::RawPathParamsRejection;
    use axum::extract::{FromRequestParts, RawPathParams};
    use axum::http::request::Parts;
    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
    use std::collections::BTreeMap;
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use std::io;

    /// Prefix of the WebSocket subprotocol that carries a session token.
    ///
    /// A browser cannot set `Authorization` on an upgrade, so it offers
    /// `vox.bearer.<token>` instead. The same value as
    /// `architect::connect::ws::BEARER_SUBPROTOCOL_PREFIX`, the client side.
    pub const BEARER_SUBPROTOCOL_PREFIX: &str = "vox.bearer.";

    /// The request line, headers and matched path params of a call: what
    /// an HTTP-only service method takes as an argument, and what every
    /// `EngineHost` protocol mount's hook decides on.
    #[derive(Clone, Debug, Default)]
    pub struct RequestHead {
        pub method: Method,
        pub uri: Uri,
        pub headers: HeaderMap,
        /// The matched route's path params, percent-decoded.
        pub params: BTreeMap<String, String>,
    }

    impl RequestHead {
        /// A head from its parts: what a test, or a transport other than
        /// axum, hands a hook. The method is `GET`.
        #[must_use]
        pub fn new(
            params: BTreeMap<String, String>,
            headers: HeaderMap,
            path: impl AsRef<str>,
        ) -> Self {
            Self {
                method: Method::GET,
                uri: Uri::try_from(path.as_ref()).unwrap_or_default(),
                headers,
                params,
            }
        }

        /// A head from axum's raw path params, headers and URI.
        pub(crate) fn from_raw(params: &RawPathParams, headers: HeaderMap, uri: &Uri) -> Self {
            Self {
                method: Method::GET,
                uri: uri.clone(),
                headers,
                params: params
                    .iter()
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect(),
            }
        }

        /// The path param `name` (`slug` in `/org/{slug}/vox`), percent-decoded.
        #[must_use]
        pub fn param(&self, name: &str) -> Option<&str> {
            self.params.get(name).map(String::as_str)
        }

        /// Every path param of the matched route.
        #[must_use]
        pub const fn params(&self) -> &BTreeMap<String, String> {
            &self.params
        }

        /// The request headers.
        #[must_use]
        pub const fn headers(&self) -> &HeaderMap {
            &self.headers
        }

        /// The request path as sent (not percent-decoded).
        #[must_use]
        pub fn path(&self) -> &str {
            self.uri.path()
        }

        /// A header as text; `None` when absent or not visible ASCII.
        #[must_use]
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers.get(name).and_then(|v| v.to_str().ok())
        }

        /// The session token the caller presented: `Authorization: Bearer …`,
        /// else a `vox.bearer.…` WebSocket subprotocol (how a browser sends
        /// it). `None` when neither carries a non-empty token.
        #[must_use]
        pub fn bearer(&self) -> Option<&str> {
            let from_header = self
                .header(header::AUTHORIZATION.as_str())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(str::trim)
                .filter(|v| !v.is_empty());
            from_header.or_else(|| {
                self.headers
                    .get_all(header::SEC_WEBSOCKET_PROTOCOL)
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .flat_map(|v| v.split(','))
                    .filter_map(|p| p.trim().strip_prefix(BEARER_SUBPROTOCOL_PREFIX))
                    .find(|t| !t.is_empty())
            })
        }

        /// The query string, without the `?`.
        #[must_use]
        pub fn query(&self) -> Option<&str> {
            self.uri.query()
        }
    }

    /// A [`RequestHead`] is also an axum extractor, for a protocol adapter
    /// built outside this crate (`architect-mcp`'s `McpHost::mcp_at`).
    impl<S: Send + Sync> FromRequestParts<S> for RequestHead {
        type Rejection = RawPathParamsRejection;

        async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
            let params = RawPathParams::from_request_parts(parts, state).await?;
            let mut head = Self::from_raw(&params, parts.headers.clone(), &parts.uri);
            head.method = parts.method.clone();
            Ok(head)
        }
    }

    /// A request body as it arrives: a `futures::Stream` of chunks.
    ///
    /// Capped at the route's `limit` (`DEFAULT_BODY_LIMIT` without one): a
    /// declared `Content-Length` over it is refused with `413` before the
    /// method runs, and a body that grows past it ends the stream with an
    /// `io::ErrorKind::FileTooLarge` error, which the method maps to its
    /// own error type.
    pub struct BodyStream {
        inner: BodyDataStream,
        limit: usize,
        seen: usize,
        done: bool,
    }

    impl BodyStream {
        pub(crate) fn new(body: Body, limit: usize) -> Self {
            Self {
                inner: body.into_data_stream(),
                limit,
                seen: 0,
                done: false,
            }
        }

        /// The most bytes this stream yields before it fails.
        #[must_use]
        pub const fn limit(&self) -> usize {
            self.limit
        }

        /// The next chunk; `None` at the end.
        pub async fn chunk(&mut self) -> Option<io::Result<Bytes>> {
            futures::StreamExt::next(self).await
        }

        /// Read the whole body into memory.
        pub async fn to_vec(mut self) -> io::Result<Vec<u8>> {
            let mut out = Vec::new();
            while let Some(chunk) = self.chunk().await {
                out.extend_from_slice(&chunk?);
            }
            Ok(out)
        }
    }

    impl futures::Stream for BodyStream {
        type Item = io::Result<Bytes>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.done {
                return Poll::Ready(None);
            }
            let next = match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(next) => next,
            };
            Poll::Ready(match next {
                Some(Ok(chunk)) => {
                    self.seen = self.seen.saturating_add(chunk.len());
                    if self.seen > self.limit {
                        self.done = true;
                        Some(Err(io::Error::new(
                            io::ErrorKind::FileTooLarge,
                            format!(
                                "the request body is over this route's {}-byte limit",
                                self.limit
                            ),
                        )))
                    } else {
                        Some(Ok(chunk))
                    }
                }
                Some(Err(error)) => {
                    self.done = true;
                    Some(Err(io::Error::other(error)))
                }
                None => {
                    self.done = true;
                    None
                }
            })
        }
    }

    type ReadFuture = Pin<Box<dyn Future<Output = io::Result<Body>> + Send>>;
    type Read = Box<dyn FnOnce(u64, u64) -> ReadFuture + Send>;

    /// A byte response: `total` bytes of `mime`, read one window at a time.
    ///
    /// The handler resolves the request's `Range` against `total` and calls
    /// `read(offset, len)` once with exactly the selected window
    /// (`(0, total)` without a range), never for a `416`. A failed read is a
    /// `500`.
    pub struct ByteBody {
        total: u64,
        mime: String,
        read: Read,
    }

    impl core::fmt::Debug for ByteBody {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("ByteBody")
                .field("total", &self.total)
                .field("mime", &self.mime)
                .finish_non_exhaustive()
        }
    }

    impl ByteBody {
        /// Bytes read by `read(offset, len)`: a blob store, an object store,
        /// a database page. Hand back a buffered or a streamed [`Body`]
        /// (`ranged::reader_body`, `Body::from_stream`).
        pub fn new<F, Fut>(total: u64, mime: impl Into<String>, read: F) -> Self
        where
            F: FnOnce(u64, u64) -> Fut + Send + 'static,
            Fut: Future<Output = io::Result<Body>> + Send + 'static,
        {
            Self {
                total,
                mime: mime.into(),
                read: Box::new(move |offset, len| Box::pin(read(offset, len))),
            }
        }

        /// Bytes already in memory; a range shares the buffer.
        pub fn from_bytes(bytes: impl Into<Bytes>, mime: impl Into<String>) -> Self {
            let bytes: Bytes = bytes.into();
            let total = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            Self::new(total, mime, move |offset, len| async move {
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                let end = start
                    .saturating_add(usize::try_from(len).unwrap_or(usize::MAX))
                    .min(bytes.len());
                Ok(Body::from(bytes.slice(start..end)))
            })
        }

        /// A file on disk, streamed; its length is read now.
        #[cfg(not(target_arch = "wasm32"))]
        pub async fn file(
            path: impl AsRef<std::path::Path>,
            mime: impl Into<String>,
        ) -> io::Result<Self> {
            use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
            let mut file = tokio::fs::File::open(path).await?;
            let total = file.metadata().await?.len();
            Ok(Self::new(total, mime, move |offset, len| async move {
                file.seek(io::SeekFrom::Start(offset)).await?;
                Ok(super::super::ranged::reader_body(file.take(len)))
            }))
        }

        /// The whole resource's length.
        #[must_use]
        pub const fn total(&self) -> u64 {
            self.total
        }

        /// The content type.
        #[must_use]
        pub fn mime(&self) -> &str {
            &self.mime
        }
    }

    /// Serve `body` for a request whose `Range` header was `range`: `200`,
    /// `206` or `416`, via `ranged::serve_body`.
    pub async fn reply_bytes(range: Option<HeaderValue>, body: ByteBody) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(range) = range {
            headers.insert(header::RANGE, range);
        }
        let ByteBody { total, mime, read } = body;
        match super::super::ranged::serve_body(&headers, total, &mime, read).await {
            Ok(response) => response,
            Err(error) => plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "read",
                &format!("could not read the body: {error}"),
            ),
        }
    }

    /// [`reply_bytes`] for a fallible method: the error as the usual
    /// envelope.
    pub async fn respond_bytes<E>(
        range: Option<HeaderValue>,
        result: Result<ByteBody, E>,
    ) -> Response
    where
        E: HttpError + facet::Facet<'static>,
    {
        match result {
            Ok(body) => reply_bytes(range, body).await,
            Err(error) => err(error),
        }
    }
}

/// The inert twin: the same names, so a trait naming them compiles
/// without `http`.
#[cfg(not(feature = "http"))]
mod imp {
    use super::super::Response;

    /// The request line and headers of a call (shape only without `http`).
    #[derive(Clone, Debug, Default)]
    pub struct RequestHead {
        _p: (),
    }

    impl RequestHead {
        #[must_use]
        pub const fn param(&self, _name: &str) -> Option<&str> {
            None
        }
        #[must_use]
        pub const fn path(&self) -> &str {
            ""
        }
        #[must_use]
        pub const fn header(&self, _name: &str) -> Option<&str> {
            None
        }
        #[must_use]
        pub const fn bearer(&self) -> Option<&str> {
            None
        }
        #[must_use]
        pub const fn query(&self) -> Option<&str> {
            None
        }
    }

    /// A streamed request body (shape only without `http`).
    pub struct BodyStream {
        _p: (),
    }

    /// A byte response (shape only without `http`).
    #[derive(Debug)]
    pub struct ByteBody {
        _p: (),
    }

    pub fn reply_bytes<R>(
        _range: R,
        _body: ByteBody,
    ) -> impl core::future::Future<Output = Response> {
        core::future::ready(Response)
    }

    pub fn respond_bytes<R, E>(
        _range: R,
        _result: Result<ByteBody, E>,
    ) -> impl core::future::Future<Output = Response> {
        core::future::ready(Response)
    }
}

#[cfg(feature = "http")]
pub use imp::BEARER_SUBPROTOCOL_PREFIX;
pub use imp::{BodyStream, ByteBody, RequestHead, reply_bytes, respond_bytes};
