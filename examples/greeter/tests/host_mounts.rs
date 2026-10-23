//! Protocol mounts that live outside `architect::host` itself, on a real
//! `EngineHost`: MCP from `architect_mcp::host::McpHost` (`.mcp`,
//! `.mcp_at`) and `.otlp_forward` to a fake collector.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use architect::LayerRouter;
use architect::action::{ActionCaller, ActionList};
use architect::host::{EngineHost, RequestHead};
use architect::http::axum::http::StatusCode;
use architect_mcp::Server;
use architect_mcp::host::{McpHost as _, McpTarget};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

async fn serve(app: architect::http::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await });
    at
}

/// One raw HTTP/1.1 request: the status code and the body.
async fn send(
    at: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(at).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {at}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    let status = reply
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap();
    let body = reply.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
    (status, body)
}

const PING: &str = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
const JSON: (&str, &str) = ("Content-Type", "application/json");

fn server() -> Arc<Server> {
    Arc::new(Server::new("Greeter", "0", ActionList::new()))
}

#[tokio::test]
async fn mcp_mounts_per_path_with_a_resolver_and_on_loopback() {
    let org_server = server();
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .mcp_at("/org/{slug}/mcp", move |head: RequestHead| {
            let org_server = Arc::clone(&org_server);
            async move {
                if head.param("slug") != Some("acme") {
                    return Err(StatusCode::NOT_FOUND);
                }
                match head.bearer() {
                    Some("alice") => Ok(McpTarget::new(org_server, ActionCaller::app())),
                    _ => Err(StatusCode::UNAUTHORIZED),
                }
            }
        })
        .mcp(
            Server::new("Local", "0", ActionList::new()),
            Some("local".into()),
        )
        .into_app();
    let at = serve(app).await;

    let alice = [JSON, ("Authorization", "Bearer alice")];
    let (status, body) = send(at, "POST", "/org/acme/mcp", &alice, PING).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""result":{}"#), "{body}");
    let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert_eq!(send(at, "POST", "/org/acme/mcp", &alice, note).await.0, 202);
    assert_eq!(
        send(at, "POST", "/org/acme/mcp", &[JSON], PING).await.0,
        401
    );
    assert_eq!(
        send(at, "POST", "/org/other/mcp", &alice, PING).await.0,
        404
    );
    assert_eq!(send(at, "GET", "/org/acme/mcp", &alice, "").await.0, 405);

    // `.mcp`: the loopback guard, unchanged.
    let local = [JSON, ("Authorization", "Bearer local")];
    assert_eq!(send(at, "POST", "/mcp", &local, PING).await.0, 200);
    assert_eq!(send(at, "POST", "/mcp", &[JSON], PING).await.0, 401);
    let evil = [
        JSON,
        ("Authorization", "Bearer local"),
        ("Origin", "https://evil.example"),
    ];
    assert_eq!(send(at, "POST", "/mcp", &evil, PING).await.0, 403);
}

/// What the fake collector saw: path, content type, body.
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

async fn collector() -> (SocketAddr, Seen) {
    use architect::http::axum::{body::Bytes, http::HeaderMap, http::Uri, routing::post};
    let seen: Seen = Arc::default();
    let log = Arc::clone(&seen);
    let app = architect::http::Router::new().route(
        "/v1/{signal}",
        post(move |uri: Uri, headers: HeaderMap, body: Bytes| {
            let log = Arc::clone(&log);
            async move {
                let content_type = headers["content-type"].to_str().unwrap().to_owned();
                let body = String::from_utf8(body.to_vec()).unwrap();
                log.lock()
                    .unwrap()
                    .push((uri.path().to_owned(), content_type, body));
                (StatusCode::OK, "partial_success")
            }
        }),
    );
    (serve(app).await, seen)
}

async fn ingest_only(head: RequestHead) -> Result<(), StatusCode> {
    match head.bearer() {
        Some("ingest") => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

#[tokio::test]
async fn otlp_forward_passes_the_three_signals_through_once_authorized() {
    let (upstream, seen) = collector().await;
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .otlp_forward("/otlp", &format!("http://{upstream}/"), ingest_only)
        .into_app();
    let at = serve(app).await;

    let ingest = [JSON, ("Authorization", "Bearer ingest")];
    let (status, body) = send(at, "POST", "/otlp/v1/traces", &ingest, r#"{"spans":1}"#).await;
    assert_eq!((status, body.as_str()), (200, "partial_success"));
    let protobuf = [("Authorization", "Bearer ingest")];
    assert_eq!(
        send(at, "POST", "/otlp/v1/logs", &protobuf, "pb").await.0,
        200
    );
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            (
                "/v1/traces".into(),
                "application/json".into(),
                r#"{"spans":1}"#.into()
            ),
            (
                "/v1/logs".into(),
                "application/x-protobuf".into(),
                "pb".into()
            ),
        ]
    );

    assert_eq!(
        send(at, "POST", "/otlp/v1/secrets", &ingest, "x").await.0,
        404
    );
    assert_eq!(
        send(at, "POST", "/otlp/v1/metrics", &[JSON], "x").await.0,
        401
    );
    assert_eq!(seen.lock().unwrap().len(), 2, "refused requests never left");

    // A collector that is not there: 502, not a hang or a 500.
    let gone = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = gone.local_addr().unwrap();
    drop(gone);
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .otlp_forward("/otlp", &format!("http://{dead}"), ingest_only)
        .into_app();
    let at = serve(app).await;
    assert_eq!(
        send(at, "POST", "/otlp/v1/metrics", &ingest, "x").await.0,
        502
    );
}
