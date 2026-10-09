//! MCP's Streamable HTTP transport, for a running app.
//!
//! `POST /mcp` takes one JSON-RPC message and answers with its reply as
//! JSON — `202 Accepted` and nothing for a notification. There is no
//! server-to-client stream (`GET` is `405`): a model follows the app
//! through the `watch` tool, which long-polls the Action List's events.
//!
//! An app serving this on localhost hands a model its whole Action List,
//! so two things stand in the way of anything else on the machine:
//!
//! - **A bearer token**, when one is given: `Authorization: Bearer …` must
//!   carry it.
//! - **The origin**: a request a browser makes carries an `Origin`, and
//!   one that is not localhost is refused — the MCP specification's guard
//!   against a web page reaching a local server through DNS rebinding.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;

use crate::Server;

struct Shared {
    server: Server,
    token: Option<String>,
}

/// `POST /mcp` serving `server`, guarded by `token` when there is one.
pub fn router(server: Server, token: Option<String>) -> Router {
    let shared = Arc::new(Shared { server, token });
    Router::new().route(
        "/mcp",
        post(message)
            .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
            .with_state(shared),
    )
}

/// Serve `server` on `listener` until the task is dropped: what an app
/// spawns on its runtime to be driven live.
///
/// # Errors
///
/// The listener failed.
pub async fn serve(
    listener: tokio::net::TcpListener,
    server: Server,
    token: Option<String>,
) -> std::io::Result<()> {
    axum::serve(listener, router(server, token)).await
}

async fn message(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok())
        && !is_local(origin)
    {
        return (StatusCode::FORBIDDEN, "not from this machine").into_response();
    }
    if let Some(token) = &shared.token {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|a| a.to_str().ok())
            .and_then(|a| a.strip_prefix("Bearer "));
        if given != Some(token.as_str()) {
            return (StatusCode::UNAUTHORIZED, "a bearer token is needed").into_response();
        }
    }
    let Ok(text) = String::from_utf8(body.to_vec()) else {
        return (StatusCode::BAD_REQUEST, "not UTF-8").into_response();
    };
    // `watch` may wait for the app: off the async threads.
    let handled = tokio::task::spawn_blocking(move || shared.server.handle(&text)).await;
    match handled {
        Ok(Some(reply)) => ([(header::CONTENT_TYPE, "application/json")], reply).into_response(),
        Ok(None) => StatusCode::ACCEPTED.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Whether an `Origin` is this machine.
fn is_local(origin: &str) -> bool {
    let host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    matches!(host, "localhost" | "127.0.0.1" | "[") || origin.contains("://[::1]")
}

#[cfg(test)]
mod tests {
    use super::{is_local, router};
    use crate::Server;
    use architect::action::ActionList;

    #[test]
    fn only_this_machine_is_local() {
        assert!(is_local("http://localhost:3000"));
        assert!(is_local("http://127.0.0.1"));
        assert!(is_local("http://[::1]:8080"));
        assert!(!is_local("https://evil.example"));
        assert!(!is_local("http://localhost.evil.example"));
    }

    /// Without the token, refused; with it, answered; a notification is
    /// accepted with nothing; a page from elsewhere is refused.
    #[tokio::test]
    async fn the_token_and_the_origin_guard_it() {
        let app = router(
            Server::new("Test", "0", ActionList::new()),
            Some("secret".into()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bound");
        let at = listener.local_addr().expect("an address");
        tokio::spawn(async move { axum::serve(listener, app).await });
        let post = |body: &'static str,
                    token: Option<&'static str>,
                    origin: Option<&'static str>| async move {
            let mut stream = tokio::net::TcpStream::connect(at).await.expect("connected");
            let mut request = format!(
                "POST /mcp HTTP/1.1\r\nHost: {at}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            if let Some(t) = token {
                request.push_str(&format!("Authorization: Bearer {t}\r\n"));
            }
            if let Some(o) = origin {
                request.push_str(&format!("Origin: {o}\r\n"));
            }
            request.push_str("\r\n");
            request.push_str(body);
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            stream.write_all(request.as_bytes()).await.expect("sent");
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.expect("read");
            reply
        };
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        assert!(post(ping, None, None).await.starts_with("HTTP/1.1 401"));
        let ok = post(ping, Some("secret"), None).await;
        assert!(
            ok.starts_with("HTTP/1.1 200") && ok.contains(r#""result":{}"#),
            "{ok}"
        );
        let note = post(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            Some("secret"),
            None,
        )
        .await;
        assert!(note.starts_with("HTTP/1.1 202"), "{note}");
        let elsewhere = post(ping, Some("secret"), Some("https://evil.example")).await;
        assert!(elsewhere.starts_with("HTTP/1.1 403"), "{elsewhere}");
    }
}
