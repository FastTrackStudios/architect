//! `architect::http` — the HTTP+JSON face of an `#[architect::rpc]` service.
//!
//! vox is the native wire for the Rust apps. A browser `fetch`, a curl
//! in a shell, a webhook, or a language with no vox binding needs the
//! same service reachable as plain HTTP with JSON bodies. Nobody should
//! hand-write that mount — the derive emits it from the trait, exactly
//! as it emits the vox client and dispatcher:
//!
//! ```text
//! #[architect::service]              →  POST /greeter/greet
//! pub trait Greeter {                    body:  {"name": "…"}      (one key per argument)
//!     async fn greet(&self, name: String) -> Result<String, GreetError>;
//! }                                      200:   "hello …"          (the return value as JSON)
//!                                        4xx:   {"code","message","error"}
//! ```
//!
//! Paths are `/<prefix>/<method-kebab>`: the prefix is the trait name in
//! kebab-case minus a trailing `Service` (`AuthService` → `/auth`), or
//! whatever `#[architect::service(path = "…")]` says.
//!
//! A method may name its own tail — `#[http(path = "sign-in/email")]` —
//! which is the only way a path nests. Derived from the identifier it
//! can be one flat segment and no more: `sign_in_email_password` gives
//! `/auth/sign-in-email-password`. A URL is a published contract, and it
//! should not move because somebody renamed a Rust method, so the
//! declaration carries it. Only the HTTP face is affected — the vox
//! method name, the generated client methods and the schema stamp all
//! stay the identifier.
//!
//! What the derive emits under the consumer's `http` feature, per trait
//! module:
//!
//! - `http::router(backend) -> axum::Router` — every method as a route.
//! - `http::<Method>Args` — the named-field argument struct each route
//!   decodes its body into. An argument called `token` may also arrive as
//!   `Authorization: Bearer …`, so a browser can keep the credential in a
//!   header exactly as it does for vox metadata.
//! - `impl BindHttp<S> for Service` — so a `layers![…]` bundle mounts its
//!   HTTP face with `Layer::provide_http(&backend)` the way `.provide(backend)`
//!   mounts the vox one.
//!
//! and under `http-client`:
//!
//! - `<Trait>HttpClient` — one async method per trait method, returning
//!   `Result<T, ClientError<E>>`: the same envelope the vox client's
//!   errors fold into, so a screen written against one transport reads
//!   errors identically on the other.
//!
//! Errors: a method's `E` says how it maps to HTTP by implementing
//! [`HttpError`] (status + stable code). The body carries the typed error
//! as JSON too, so the generated client hands the caller back the real
//! `E`, not a string.
//!
//! # One facade, every build
//!
//! The derive emits the same code whether or not this crate was built
//! with `http` / `http-client`. This module always exports the names it
//! uses — `Router`, `MethodRouter`, `State`, `Response`, `post`, `get`,
//! `Call`, `respond`, `HttpClient`, `EventStream` — as axum's and
//! reqwest's under the features, and as inert stubs otherwise (a stub
//! router mounts nothing; a stub client answers every call with a
//! `Transport` error saying the feature is off). That is what lets a
//! proto crate forward **no** cargo features: the app turns `http` on in
//! `architect` and the routes appear.

use core::fmt::Display;

/// How an application error appears on the HTTP face.
///
/// Implemented by the error type of every `Result`-returning method the
/// derive mounts. The defaults are a safe generic: `400` with code
/// `"error"`. Override `status` for the errors that are really `401` /
/// `403` / `404` / `409`, and `code` for the ones a client switches on.
pub trait HttpError: Display {
    /// The response status. Anything below 400 is coerced to 400: an
    /// error that reports success would be silently mis-handled by every
    /// client.
    fn status(&self) -> u16 {
        400
    }

    /// A stable, machine-readable identifier (`"invalid_credentials"`).
    /// `message` is the `Display` form and may change; this must not.
    fn code(&self) -> &'static str {
        "error"
    }
}

/// Every `RepoError` the entity derive returns has an obvious status.
impl HttpError for crate::RepoError {
    fn status(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::InvalidInput(_) => 400,
            Self::Conflict(_) => 409,
            Self::Internal(_) => 500,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::InvalidInput(_) => "invalid_input",
            Self::Conflict(_) => "conflict",
            Self::Internal(_) => "internal",
        }
    }
}

/// The failure envelope on the wire.
///
/// `code` and `message` are for humans and for clients that only speak
/// JSON; `error` is the typed `E`, so the generated client can decode it
/// back into the same enum the vox client would have returned.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct ErrorBody<E> {
    pub code: String,
    pub message: String,
    pub error: E,
}

/// The one path scheme every generated route and client agrees on.
///
/// `/<prefix>/<method-kebab>`: `prefix` is the service's mount prefix
/// (`auth`); `method` may be given in `snake_case` and is kebab-cased.
/// Public so a test or a reverse proxy can compute a path without
/// depending on the emitted constants.
#[must_use]
pub fn path(prefix: &str, method: &str) -> String {
    format!("/{}/{}", prefix.trim_matches('/'), method.replace('_', "-"))
}

// ── Server side ────────────────────────────────────────────────────────

/// The route table a bundle binds into before it becomes a router.
///
/// Keyed by path, later inserts replace earlier ones — which is how a
/// merged mock overrides a service's HTTP face the way it overrides the
/// vox one (an axum `Router::merge` would panic on the duplicate).
#[derive(Default)]
pub struct HttpRoutes {
    // Zero-sized in a build without `http` (the stub router), hence the allow.
    #[allow(clippy::zero_sized_map_values)]
    routes: std::collections::BTreeMap<String, MethodRouter>,
}

impl HttpRoutes {
    /// Mount (or replace) the handler at `path`.
    pub fn route(&mut self, path: &str, handler: MethodRouter) -> &mut Self {
        self.routes.insert(path.to_owned(), handler);
        self
    }

    /// The paths mounted so far, sorted.
    #[must_use]
    pub fn paths(&self) -> Vec<&str> {
        self.routes.keys().map(String::as_str).collect()
    }

    /// Turn the table into a router.
    #[must_use]
    pub fn into_router(self) -> Router {
        let mut router = Router::new();
        for (path, handler) in self.routes {
            router = router.route(&path, handler);
        }
        router
    }
}

/// Bind a backend's HTTP face into a route table — the HTTP twin of [`crate::Bind`].
///
/// Emitted for every service token; implemented here for the layer cells
/// and for [`crate::Mounted`], so a whole `layers![…]` bundle (mocks
/// included) binds at once.
pub trait BindHttp<B> {
    fn bind_http(&self, backend: &B, routes: &mut HttpRoutes);
}

#[cfg(feature = "vox")]
impl<B> BindHttp<B> for crate::Empty {
    fn bind_http(&self, _backend: &B, _routes: &mut HttpRoutes) {}
}

#[cfg(feature = "vox")]
impl<B, S, R> BindHttp<B> for crate::Cons<S, R>
where
    S: BindHttp<B>,
    R: BindHttp<B>,
{
    fn bind_http(&self, backend: &B, routes: &mut HttpRoutes) {
        // Tail first, head last — the same order as the vox bind, so a
        // merged mock at the head replaces the bundle's routes.
        self.rest().bind_http(backend, routes);
        self.svc().bind_http(backend, routes);
    }
}

#[cfg(feature = "vox")]
impl<B> BindHttp<B> for crate::Mounted {
    fn bind_http(&self, _backend: &B, routes: &mut HttpRoutes) {
        self.bind_http_into(routes);
    }
}

/// The bearer-fallback rule the generated handlers apply to an argument
/// named `token`: the body wins when it says something, the header fills
/// in when it does not.
#[must_use]
pub fn token_or_bearer(from_body: String, bearer: Option<String>) -> String {
    if from_body.is_empty() {
        bearer.unwrap_or_default()
    } else {
        from_body
    }
}

#[cfg(feature = "http")]
mod server {
    use super::{ErrorBody, HttpError};
    use axum::body::Bytes;
    use axum::extract::{FromRequest, Request};
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;

    pub use axum;
    pub use axum::Router;
    pub use axum::extract::State;
    pub use axum::response::Response;
    pub use axum::routing::{get, post};
    /// A route's handler set, state already applied.
    pub type MethodRouter = axum::routing::MethodRouter<()>;

    /// One decoded call: the argument struct plus the bearer token the
    /// request carried, if any. The generated handlers take this as their
    /// extractor.
    ///
    /// The body may be empty for a method with no arguments — `{}` and no
    /// body at all decode the same way. The content type is not checked:
    /// a `fetch` that forgot the header should still work, and a body that
    /// is not JSON fails at decode with a `422` that says why.
    pub struct Call<A> {
        pub args: A,
        pub bearer: Option<String>,
    }

    impl<S, A> FromRequest<S> for Call<A>
    where
        S: Send + Sync,
        A: facet::Facet<'static>,
    {
        type Rejection = Response;

        async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
            let bearer = req
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
            let bytes = Bytes::from_request(req, state)
                .await
                .map_err(IntoResponse::into_response)?;
            let body: &[u8] = if bytes.is_empty() { b"{}" } else { &bytes };
            let args = facet_json::from_slice::<A>(body).map_err(|err| {
                plain_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_body",
                    &format!("could not decode request body: {err}"),
                )
            })?;
            Ok(Self { args, bearer })
        }
    }

    /// A `Result`-returning method's reply: `200` + the value, or the
    /// error's status + [`ErrorBody`].
    pub fn respond<T, E>(result: Result<T, E>) -> Response
    where
        T: facet::Facet<'static>,
        E: HttpError + facet::Facet<'static>,
    {
        match result {
            Ok(value) => ok(&value),
            Err(error) => err(error),
        }
    }

    /// An infallible method's reply: always `200` + the value.
    pub fn ok<T>(value: &T) -> Response
    where
        T: facet::Facet<'static>,
    {
        match facet_json::to_vec(value) {
            Ok(body) => json_response(StatusCode::OK, body),
            Err(error) => plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "encode",
                &format!("could not encode response: {error}"),
            ),
        }
    }

    /// A typed application error as an HTTP reply.
    pub fn err<E>(error: E) -> Response
    where
        E: HttpError + facet::Facet<'static>,
    {
        let status = StatusCode::from_u16(error.status().max(400))
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ErrorBody {
            code: error.code().to_owned(),
            message: error.to_string(),
            error,
        };
        match facet_json::to_vec(&body) {
            Ok(bytes) => json_response(status, bytes),
            Err(encode) => plain_error(
                status,
                &body.code,
                &format!(
                    "{} (error body could not be encoded: {encode})",
                    body.message
                ),
            ),
        }
    }

    /// An envelope with no typed payload — for failures that happen
    /// before the method runs (a body that would not decode).
    #[must_use]
    pub fn plain_error(status: StatusCode, code: &str, message: &str) -> Response {
        let body = ErrorBody {
            code: code.to_owned(),
            message: message.to_owned(),
            error: (),
        };
        let bytes = facet_json::to_vec(&body).unwrap_or_default();
        json_response(status, bytes)
    }

    fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
        (
            status,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body,
        )
            .into_response()
    }

    /// Serve a subscription as a Server-Sent-Events response.
    ///
    /// `subscribe` receives a local [`EventSink`](crate::EventSink) and
    /// hands it to whatever produces events — `hub.attach(sink)`, a
    /// filtered `*_attach(filter, sink)`, a snapshot-then-changes
    /// `begin_attach` / `complete_attach`. Every event then leaves as one
    /// `data: <json>` frame; the stream ends when the producer drops the
    /// sink, and the producer sees the sink close when the client goes
    /// away.
    #[cfg(feature = "vox")]
    pub fn sse<T, F>(subscribe: F) -> Response
    where
        T: facet::Facet<'static> + Send + 'static,
        F: FnOnce(crate::EventSink<T>),
    {
        let (sink, rx) = crate::EventSink::local(64);
        subscribe(sink);
        sse_from(rx)
    }

    /// [`sse`] over a receiver the caller already holds — for producers
    /// that need the sink before the response can start (a
    /// snapshot-then-changes `begin_attach` / `complete_attach`).
    #[cfg(feature = "vox")]
    #[must_use]
    pub fn sse_from<T>(rx: async_channel::Receiver<T>) -> Response
    where
        T: facet::Facet<'static> + Send + 'static,
    {
        use futures::StreamExt as _;
        let frames = rx.map(|event| {
            let payload = facet_json::to_vec(&event).unwrap_or_default();
            let mut frame = Vec::with_capacity(payload.len().saturating_add(8));
            frame.extend_from_slice(b"data: ");
            frame.extend_from_slice(&payload);
            frame.extend_from_slice(b"\n\n");
            Ok::<_, std::convert::Infallible>(Bytes::from(frame))
        });
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/event-stream"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            axum::body::Body::from_stream(frames),
        )
            .into_response()
    }
}

/// The inert twin of the server half: the same names, mounting nothing.
/// Exists so derive-emitted routers compile in a build without `http`
/// (a wasm client, a proto crate checked on its own).
#[cfg(not(feature = "http"))]
mod server {
    use core::marker::PhantomData;

    /// A route table that goes nowhere.
    #[derive(Default, Debug, Clone)]
    pub struct Router;
    impl Router {
        #[must_use]
        pub const fn new() -> Self {
            Self
        }
        #[must_use]
        pub const fn route(self, _path: &str, _handler: MethodRouter) -> Self {
            self
        }
        #[must_use]
        pub const fn merge(self, _other: Self) -> Self {
            self
        }
        #[must_use]
        pub fn with_state<S>(self, _state: S) -> Self {
            self
        }
        #[must_use]
        pub const fn nest(self, _path: &str, _other: Self) -> Self {
            self
        }
    }
    /// A handler set that holds nothing.
    #[derive(Default, Debug, Clone)]
    pub struct MethodRouter;
    impl MethodRouter {
        #[must_use]
        pub fn get<H>(self, _handler: H) -> Self {
            self
        }
        #[must_use]
        pub fn post<H>(self, _handler: H) -> Self {
            self
        }
        #[must_use]
        pub fn with_state<S>(self, _state: S) -> Self {
            self
        }
    }
    #[must_use]
    pub fn post<H>(_handler: H) -> MethodRouter {
        MethodRouter
    }
    #[must_use]
    pub fn get<H>(_handler: H) -> MethodRouter {
        MethodRouter
    }
    /// axum's `State` extractor, shape only.
    pub struct State<T>(pub T);
    /// axum's `Response`, shape only.
    #[derive(Default, Debug, Clone, Copy)]
    pub struct Response;
    /// The decoded-call extractor, shape only.
    pub struct Call<A> {
        pub args: A,
        pub bearer: Option<String>,
        _p: PhantomData<A>,
    }
    #[must_use]
    pub fn respond<T, E>(_result: Result<T, E>) -> Response {
        Response
    }
    #[must_use]
    pub const fn ok<T>(_value: &T) -> Response {
        Response
    }
    #[must_use]
    pub fn err<E>(_error: E) -> Response {
        Response
    }
    #[cfg(feature = "vox")]
    #[must_use]
    pub fn sse<T, F>(_subscribe: F) -> Response
    where
        F: FnOnce(crate::EventSink<T>),
    {
        Response
    }
    #[cfg(feature = "vox")]
    #[must_use]
    pub fn sse_from<T>(_rx: async_channel::Receiver<T>) -> Response {
        Response
    }
}

pub use server::{Call, MethodRouter, Response, Router, State, err, get, ok, post, respond};
#[cfg(feature = "http")]
pub use server::{axum, plain_error};
#[cfg(feature = "vox")]
pub use server::{sse, sse_from};

#[cfg(any(feature = "http", feature = "http-client"))]
pub use facet_json;

// ── Client side (`http-client`) ────────────────────────────────────────

/// A subscription as the generated HTTP clients hand it back: the
/// decoded events, or the transport failure that ended the stream.
#[cfg(not(target_arch = "wasm32"))]
pub type EventStream<T> =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<T, crate::ClientError<()>>> + Send>>;
/// Wasm: `fetch`'s body stream is single-threaded, so no `Send`.
#[cfg(target_arch = "wasm32")]
pub type EventStream<T> =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<T, crate::ClientError<()>>>>>;

/// The inert twin of the client half: a client that fails every call
/// with a `Transport` error naming the missing feature. Exists so
/// derive-emitted clients compile in a build without `http-client`.
#[cfg(not(feature = "http-client"))]
mod client {
    use super::EventStream;
    use crate::ClientError;

    #[derive(Clone, Debug)]
    pub struct HttpClient {
        base_url: String,
        bearer: Option<String>,
    }

    impl HttpClient {
        #[must_use]
        pub fn new(base_url: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into().trim_end_matches('/').to_owned(),
                bearer: None,
            }
        }
        #[must_use]
        pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
            self.bearer = Some(token.into());
            self
        }
        #[must_use]
        pub fn without_bearer(mut self) -> Self {
            self.bearer = None;
            self
        }
        #[must_use]
        pub fn bearer(&self) -> Option<&str> {
            self.bearer.as_deref()
        }
        #[must_use]
        pub fn base_url(&self) -> &str {
            &self.base_url
        }
        fn off<E>(path: &str) -> ClientError<E> {
            ClientError::Transport {
                detail: format!("{path}: architect was built without the `http-client` feature"),
                retryable: false,
            }
        }
        pub fn call<'a, A, T, E>(
            &'a self,
            path: &str,
            _args: &A,
        ) -> impl ::core::future::Future<Output = Result<T, ClientError<E>>> + crate::MaybeSend + 'a
        where
            T: crate::MaybeSend + 'a,
            E: crate::MaybeSend + 'a,
        {
            let err = Self::off(path);
            async move { Err(err) }
        }
        pub fn stream<'a, A, T>(
            &'a self,
            path: &str,
            _args: &A,
        ) -> impl ::core::future::Future<Output = Result<EventStream<T>, ClientError<()>>>
        + crate::MaybeSend
        + 'a
        where
            T: 'a,
        {
            let err = Self::off(path);
            async move { Err(err) }
        }
    }
}

#[cfg(feature = "http-client")]
mod client {
    use super::{ErrorBody, EventStream};
    use crate::ClientError;

    /// Index just past the first complete SSE frame (`\n\n`), if any.
    fn find_frame_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(2).position(|w| w == b"\n\n")
    }

    /// The concatenated `data:` lines of one frame, per the SSE spec
    /// (multiple `data:` lines join with newlines; comments and other
    /// fields are ignored). `None` for a frame with no data.
    fn sse_data(frame: &[u8]) -> Option<Vec<u8>> {
        let mut out: Vec<u8> = Vec::new();
        let mut any = false;
        for line in frame.split(|&b| b == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(rest) = line.strip_prefix(b"data:") {
                let rest = rest.strip_prefix(b" ").unwrap_or(rest);
                if any {
                    out.push(b'\n');
                }
                out.extend_from_slice(rest);
                any = true;
            }
        }
        any.then_some(out)
    }

    /// The transport the generated `<Trait>HttpClient`s share: a base URL,
    /// an optional bearer token, and one `reqwest` client. Native and
    /// wasm (where `reqwest` rides `fetch`).
    ///
    /// Clone it per feature, the way a vox `Caller` is shared: the
    /// connection pool inside `reqwest::Client` is reference-counted.
    #[derive(Clone, Debug)]
    pub struct HttpClient {
        base_url: String,
        bearer: Option<String>,
        http: reqwest::Client,
    }

    impl HttpClient {
        /// A client rooted at `base_url` (`https://auth.example.com` or
        /// `http://127.0.0.1:8080/api`); trailing slashes are trimmed so
        /// path joins never double up.
        #[must_use]
        pub fn new(base_url: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into().trim_end_matches('/').to_owned(),
                bearer: None,
                http: reqwest::Client::new(),
            }
        }

        /// Reuse an existing `reqwest::Client` (custom timeouts, TLS, …).
        #[must_use]
        pub fn with_reqwest(mut self, http: reqwest::Client) -> Self {
            self.http = http;
            self
        }

        /// Send `Authorization: Bearer <token>` on every call.
        #[must_use]
        pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
            self.bearer = Some(token.into());
            self
        }

        /// Stop sending a bearer token.
        #[must_use]
        pub fn without_bearer(mut self) -> Self {
            self.bearer = None;
            self
        }

        /// The token this client presents, if any.
        #[must_use]
        pub fn bearer(&self) -> Option<&str> {
            self.bearer.as_deref()
        }

        /// The root every path is joined onto.
        #[must_use]
        pub fn base_url(&self) -> &str {
            &self.base_url
        }

        /// One call: `POST <base>/<path>` with `args` as the JSON body.
        ///
        /// `2xx` decodes as `T`; anything else decodes as
        /// [`ErrorBody<E>`] and becomes `ClientError::App(E)`. A reply that
        /// is neither — a proxy's HTML 502, a body that is not JSON — is a
        /// `Transport` error, retryable when the status suggests the
        /// server was not reached.
        ///
        /// The arguments are encoded before the request is built, so the
        /// returned future borrows nothing from `args` and is `Send` on
        /// native (wasm's `fetch` future is single-threaded, hence
        /// `MaybeSend`).
        pub fn call<'a, A, T, E>(
            &'a self,
            path: &str,
            args: &A,
        ) -> impl ::core::future::Future<Output = Result<T, ClientError<E>>> + crate::MaybeSend + 'a
        where
            A: for<'f> facet::Facet<'f>,
            T: facet::Facet<'static> + crate::MaybeSend,
            E: facet::Facet<'static> + crate::MaybeSend,
        {
            let path = path.to_owned();
            let encoded = facet_json::to_vec(args).map_err(|err| ClientError::Transport {
                detail: format!("encode {path} args: {err}"),
                retryable: false,
            });
            async move {
                let body = encoded?;
                self.call_encoded(&path, body).await
            }
        }

        /// Open a subscription: `POST <base>/<path>` with `args`, reading
        /// the Server-Sent-Events reply as a stream of `T`. Ends when the
        /// server closes it; drop the stream to unsubscribe.
        ///
        /// A non-2xx reply is returned up front as a `Transport` error
        /// (subscriptions carry no typed error: the producer either
        /// accepts the sink or it does not).
        pub fn stream<'a, A, T>(
            &'a self,
            path: &str,
            args: &A,
        ) -> impl ::core::future::Future<Output = Result<EventStream<T>, ClientError<()>>>
        + crate::MaybeSend
        + 'a
        where
            A: for<'f> facet::Facet<'f>,
            T: facet::Facet<'static> + crate::MaybeSend + 'static,
        {
            let path = path.to_owned();
            let encoded = facet_json::to_vec(args).map_err(|err| ClientError::Transport {
                detail: format!("encode {path} args: {err}"),
                retryable: false,
            });
            async move {
                let body = encoded?;
                self.stream_encoded(path, body).await
            }
        }

        async fn stream_encoded<T>(
            &self,
            path: String,
            body: Vec<u8>,
        ) -> Result<EventStream<T>, ClientError<()>>
        where
            T: facet::Facet<'static> + crate::MaybeSend + 'static,
        {
            use futures::StreamExt as _;
            let mut request = self
                .http
                .post(format!("{}{path}", self.base_url))
                .header("content-type", "application/json")
                .header("accept", "text/event-stream")
                .body(body);
            if let Some(token) = &self.bearer {
                request = request.bearer_auth(token);
            }
            let response = request.send().await.map_err(|err| ClientError::Transport {
                detail: format!("{path}: {err}"),
                retryable: true,
            })?;
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(ClientError::Transport {
                    detail: format!("{path}: HTTP {}: {text}", status.as_u16()),
                    retryable: status.is_server_error(),
                });
            }
            let buffer: Vec<u8> = Vec::new();
            let pending: std::collections::VecDeque<Vec<u8>> = std::collections::VecDeque::new();
            let bytes = response.bytes_stream();
            let events = futures::stream::unfold(
                (bytes, buffer, pending, path, false),
                |(mut bytes, mut buffer, mut pending, path, mut done)| async move {
                    loop {
                        if let Some(frame) = pending.pop_front() {
                            let item = facet_json::from_slice::<T>(&frame).map_err(|err| {
                                ClientError::Transport {
                                    detail: format!("{path}: decode event: {err}"),
                                    retryable: false,
                                }
                            });
                            return Some((item, (bytes, buffer, pending, path, done)));
                        }
                        if done {
                            return None;
                        }
                        match bytes.next().await {
                            Some(Ok(chunk)) => {
                                buffer.extend_from_slice(&chunk);
                                while let Some(end) = find_frame_end(&buffer) {
                                    let frame: Vec<u8> = buffer.drain(..end).collect();
                                    // Skip the blank-line terminator.
                                    buffer.drain(..2);
                                    if let Some(data) = sse_data(&frame) {
                                        pending.push_back(data);
                                    }
                                }
                            }
                            Some(Err(err)) => {
                                done = true;
                                let item = Err(ClientError::Transport {
                                    detail: format!("{path}: stream: {err}"),
                                    retryable: true,
                                });
                                return Some((item, (bytes, buffer, pending, path, done)));
                            }
                            None => {
                                done = true;
                            }
                        }
                    }
                },
            );
            Ok(Box::pin(events))
        }

        async fn call_encoded<T, E>(&self, path: &str, body: Vec<u8>) -> Result<T, ClientError<E>>
        where
            T: facet::Facet<'static>,
            E: facet::Facet<'static>,
        {
            let mut request = self
                .http
                .post(format!("{}{path}", self.base_url))
                .header("content-type", "application/json")
                .body(body);
            if let Some(token) = &self.bearer {
                request = request.bearer_auth(token);
            }
            let response = request.send().await.map_err(|err| ClientError::Transport {
                detail: format!("{path}: {err}"),
                retryable: true,
            })?;
            let status = response.status();
            let bytes = response
                .bytes()
                .await
                .map_err(|err| ClientError::Transport {
                    detail: format!("{path}: read body: {err}"),
                    retryable: true,
                })?;
            if status.is_success() {
                return facet_json::from_slice::<T>(&bytes).map_err(|err| ClientError::Transport {
                    detail: format!("{path}: decode reply: {err}"),
                    retryable: false,
                });
            }
            match facet_json::from_slice::<ErrorBody<E>>(&bytes) {
                Ok(envelope) => Err(ClientError::App(envelope.error)),
                Err(_) => Err(ClientError::Transport {
                    detail: format!(
                        "{path}: HTTP {}: {}",
                        status.as_u16(),
                        String::from_utf8_lossy(&bytes)
                    ),
                    retryable: status.is_server_error(),
                }),
            }
        }
    }
}

pub use client::HttpClient;
