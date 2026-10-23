//! [`EngineHost::otlp_forward`]: OTLP/HTTP ingest a host forwards to a
//! collector.
//!
//! The desktop, mobile and browser builds emit the same traces, logs and
//! metrics a server does, but the collector is in-cluster on purpose.
//! Publishing it would mean an endpoint anyone can write into the trace
//! store through, with a credential of its own to provision and rotate.
//! The host proxies instead. Clients already hold a session token and
//! already talk to this host, so pointing `OTEL_EXPORTER_OTLP_ENDPOINT`
//! at `<host>/otlp` reuses its auth, TLS and CORS.

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;

use super::{EngineHost, RequestHead};

/// The path param the signal is bound to.
const SIGNAL: &str = "otlp_signal";

/// The three OTLP/HTTP signals. Nothing else is forwarded: no path is
/// passed through into an in-cluster service.
pub const OTLP_SIGNALS: [&str; 3] = ["traces", "logs", "metrics"];

struct Upstream {
    base: String,
    client: Result<reqwest::Client, String>,
}

impl EngineHost {
    /// Accept OTLP/HTTP at `at` (`/otlp`) and forward it to `upstream`,
    /// a collector's OTLP/HTTP base (`http://otel-collector:4318`).
    ///
    /// Mounts `POST {at}/v1/{traces,logs,metrics}`, the paths an `OTel`
    /// SDK builds from a base endpoint. Each request calls `authorize`
    /// with its [`RequestHead`] first; an `Err` (any [`IntoResponse`]) is
    /// the answer. Usually the check is "a real signed-in client of this
    /// host" ([`RequestHead::bearer`]), plus a static ingest token for
    /// exporters that start before anyone signs in. The body passes
    /// through untouched (protobuf or JSON) with its `Content-Type`
    /// (`application/x-protobuf` when absent). The collector's status and
    /// body come back as they are, and an unreachable collector is `502`.
    ///
    /// A host with no collector should not call this: a missing route is
    /// a clearer answer than one that always says `502`.
    #[must_use]
    pub fn otlp_forward<E, F, Fut>(self, at: &str, upstream: &str, authorize: F) -> Self
    where
        F: Fn(RequestHead) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: IntoResponse,
    {
        let upstream = Arc::new(Upstream {
            base: upstream.trim().trim_end_matches('/').to_owned(),
            client: reqwest::Client::builder()
                .build()
                .map_err(|e| e.to_string()),
        });
        let accept = move |head: RequestHead, body: Bytes| {
            let upstream = Arc::clone(&upstream);
            let signal = head
                .param(SIGNAL)
                .filter(|s| OTLP_SIGNALS.contains(s))
                .map(str::to_owned);
            let content_type = head
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/x-protobuf")
                .to_owned();
            let authorized = authorize(head);
            async move {
                let Some(signal) = signal else {
                    return (StatusCode::NOT_FOUND, "unknown OTLP signal").into_response();
                };
                if let Err(refused) = authorized.await {
                    return refused.into_response();
                }
                upstream.forward(&signal, content_type, body).await
            }
        };
        let at = at.trim_end_matches('/');
        self.extend(Router::new().route(&format!("{at}/v1/{{{SIGNAL}}}"), post(accept)))
    }
}

impl Upstream {
    async fn forward(&self, signal: &str, content_type: String, body: Bytes) -> Response {
        let client = match &self.client {
            Ok(client) => client,
            Err(e) => {
                tracing::warn!(signal, error = %e, "OTLP forward has no HTTP client");
                return (StatusCode::BAD_GATEWAY, "collector unreachable").into_response();
            }
        };
        let sent = client
            .post(format!("{}/v1/{signal}", self.base))
            .header(header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await;
        match sent {
            Ok(res) => {
                let status = res.status();
                match res.bytes().await {
                    Ok(bytes) => (status, bytes).into_response(),
                    Err(e) => {
                        tracing::warn!(signal, error = %e, "OTLP collector reply unreadable");
                        StatusCode::BAD_GATEWAY.into_response()
                    }
                }
            }
            // The failure is the host's to see; a client that cannot export
            // telemetry should degrade quietly.
            Err(e) => {
                tracing::warn!(signal, error = %e, "OTLP forward failed");
                (StatusCode::BAD_GATEWAY, "collector unreachable").into_response()
            }
        }
    }
}
