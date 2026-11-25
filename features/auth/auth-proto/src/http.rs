//! The HTTP+JSON face of auth's services, beside their vox face.
//!
//! ```text
//! #[auth_macros::service(path = "auth")]   →  POST /auth/devices/register
//! pub trait DeviceService {                    body:  {"token": "…", "registration": {…}}
//!     #[http(path = "devices/register")]       200:   the return value as JSON
//!     async fn register_device(…) -> Result<…, AuthFlowError>;
//! }                                            4xx:   {"code","message","error"}
//! ```
//!
//! A path is `/<prefix>/<tail>`: the prefix is the service's `path` (or
//! the trait name in kebab-case minus a trailing `Service`), the tail the
//! method's `#[http(path = "…")]` (or its name in kebab-case). A path is a
//! published contract (apps already shipped call it), pinned by
//! `tests/wire.rs`.
//!
//! An argument called `token` may also arrive as `Authorization: Bearer`.
//! Errors map to a status and a stable code through [`HttpError`]; the
//! body carries the typed error too ([`ErrorBody`]), so the client hands
//! back the real error, not a string.
//!
//! - `http` (server): the [`Call`] extractor and [`respond`], on axum.
//! - `http-client`: [`HttpClient`], on reqwest (native and wasm).

use core::fmt::Display;

/// `Send + Sync` natively; nothing on wasm (a browser's futures are not).
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// How an application error appears on the HTTP face: a status (below
/// 400 is taken as 400) and a stable, machine-readable code.
pub trait HttpError: Display {
    fn status(&self) -> u16 {
        400
    }
    fn code(&self) -> &'static str {
        "error"
    }
}

/// The failure envelope on the wire.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct ErrorBody<E> {
    pub code: String,
    pub message: String,
    pub error: E,
}

/// A call that failed before the service answered: the connection
/// dropped, the reply was not JSON, nothing could be reached.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("transport failure: {detail}")]
pub struct TransportError {
    pub detail: String,
    /// Whether a retry on a fresh connection may succeed.
    pub retryable: bool,
}

/// What a client call can fail with: the service's own error, or the
/// connection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError<E = String> {
    /// The service ran and returned its typed error.
    #[error("{0}")]
    App(E),
    /// The connection could not be made.
    #[error("could not connect: {0}")]
    Connect(String),
    /// The call failed in flight.
    #[error("transport failure: {detail}")]
    Transport { detail: String, retryable: bool },
}

impl<E> ClientError<E> {
    /// The service's typed error, if this is one.
    pub const fn app(&self) -> Option<&E> {
        match self {
            Self::App(e) => Some(e),
            _ => None,
        }
    }

    /// Whether this is the connection failing, not the service refusing.
    pub const fn is_infrastructure(&self) -> bool {
        !matches!(self, Self::App(_))
    }

    /// Whether a retry on a fresh connection may succeed.
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::App(_) => false,
            Self::Connect(_) => true,
            Self::Transport { retryable, .. } => *retryable,
        }
    }

    /// Fold into the service's error: a typed error passes through, a
    /// transport failure is absorbed through `From<TransportError>`.
    pub fn into_app(self) -> E
    where
        E: From<TransportError>,
    {
        match self {
            Self::App(e) => e,
            Self::Connect(detail) => E::from(TransportError {
                detail,
                retryable: true,
            }),
            Self::Transport { detail, retryable } => E::from(TransportError { detail, retryable }),
        }
    }
}

/// The vox twin of [`ClientError::into_app`].
#[cfg(feature = "vox")]
#[must_use]
pub fn vox_into_app<E>(e: vox::VoxError<E>) -> E
where
    E: From<TransportError> + Display,
{
    match e {
        vox::VoxError::User(e) => *e,
        other => E::from(TransportError {
            retryable: other.is_connection_interruption(),
            detail: other.to_string(),
        }),
    }
}

/// The bearer-fallback rule for an argument called `token`: the body
/// wins when it says something, the header fills in when it does not.
#[must_use]
pub fn token_or_bearer(from_body: String, bearer: Option<String>) -> String {
    if from_body.is_empty() {
        bearer.unwrap_or_default()
    } else {
        from_body
    }
}

// A ready `Response` is the rejection type of the extractor (axum's own
// idiom): the handler returns it as it is.
#[cfg(feature = "http")]
#[allow(clippy::result_large_err)]
mod server {
    use super::{ErrorBody, HttpError};
    use axum::body::Bytes;
    use axum::extract::{FromRequest, Request};
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::response::{IntoResponse, Response};

    fn bearer_of(headers: &HeaderMap) -> Option<String> {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    }

    /// A JSON body as the argument struct; empty means `{}`.
    fn decode_args<A: facet::Facet<'static>>(bytes: &[u8]) -> Result<A, Response> {
        let body: &[u8] = if bytes.is_empty() { b"{}" } else { bytes };
        facet_json::from_slice::<A>(body).map_err(|err| {
            plain_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_body",
                &format!("could not decode request body: {err}"),
            )
        })
    }

    /// One decoded call: the argument struct and the bearer token the
    /// request carried, if any. The content type is not checked: a body
    /// that is not JSON fails to decode with a `422` that says why.
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
            let bearer = bearer_of(req.headers());
            let bytes = Bytes::from_request(req, state)
                .await
                .map_err(IntoResponse::into_response)?;
            let args = decode_args::<A>(&bytes)?;
            Ok(Self { args, bearer })
        }
    }

    /// A method's reply: `200` and the value, or the error's status and
    /// [`ErrorBody`].
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

    /// `200` and the value.
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

    /// A typed error as an HTTP reply.
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

    /// An envelope with no typed payload, for failures before the method
    /// runs (a body that would not decode).
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
}

#[cfg(feature = "http")]
pub use axum;
#[cfg(feature = "http")]
pub use server::{Call, err, ok, plain_error, respond};

#[cfg(feature = "http-client")]
mod client {
    use super::{ClientError, ErrorBody, MaybeSend};

    /// What the `<Service>HttpClient`s share: a base URL, an optional
    /// bearer token, and one reqwest client (its pool is shared by clones).
    #[derive(Clone, Debug)]
    pub struct HttpClient {
        base_url: String,
        bearer: Option<String>,
        http: reqwest::Client,
    }

    impl HttpClient {
        /// A client rooted at `base_url` (trailing slashes trimmed).
        #[must_use]
        pub fn new(base_url: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into().trim_end_matches('/').to_owned(),
                bearer: None,
                http: reqwest::Client::new(),
            }
        }

        /// Reuse an existing reqwest client (timeouts, TLS, …).
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

        /// One call: `POST <base><path>` with `args` as the JSON body.
        /// `2xx` decodes as `T`; anything else as [`ErrorBody<E>`], which
        /// becomes `ClientError::App(E)`. A reply that is neither (a
        /// proxy's HTML 502) is a `Transport` error, retryable when the
        /// status says the server was not reached.
        pub fn call<'a, A, T, E>(
            &'a self,
            path: &str,
            args: &A,
        ) -> impl ::core::future::Future<Output = Result<T, ClientError<E>>> + MaybeSend + 'a
        where
            A: for<'f> facet::Facet<'f>,
            T: facet::Facet<'static> + MaybeSend,
            E: facet::Facet<'static> + MaybeSend,
        {
            let path = path.to_owned();
            let encoded = facet_json::to_vec(args).map_err(|err| ClientError::Transport {
                detail: format!("encode {path} args: {err}"),
                retryable: false,
            });
            async move {
                let body = encoded?;
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
                    return facet_json::from_slice::<T>(&bytes).map_err(|err| {
                        ClientError::Transport {
                            detail: format!("{path}: decode reply: {err}"),
                            retryable: false,
                        }
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
}

#[cfg(feature = "http-client")]
pub use client::HttpClient;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_token_wins_and_the_bearer_fills_in() {
        assert_eq!(token_or_bearer("a".into(), Some("b".into())), "a");
        assert_eq!(token_or_bearer(String::new(), Some("b".into())), "b");
        assert_eq!(token_or_bearer(String::new(), None), "");
    }
}
