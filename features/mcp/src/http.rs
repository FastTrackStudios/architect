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

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, post};

use architect::action::ActionCaller;

use crate::Server;

struct Shared {
    server: Arc<Server>,
    token: Option<String>,
}

/// `POST /mcp` serving `server`, guarded by `token` when there is one.
pub fn router(server: Server, token: Option<String>) -> Router {
    let shared = Arc::new(Shared {
        server: Arc::new(server),
        token,
    });
    mount("/mcp", post(message).with_state(shared))
}

/// `messages` at `path`; `GET` is `405` (there is no server-to-client
/// stream). Every MCP route is built here.
pub(crate) fn mount(path: &str, messages: MethodRouter) -> Router {
    Router::new().route(
        path,
        messages.get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
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

/// A server [`Server::serve_loopback`] started: where it is, the token it
/// wants, and the task serving it.
#[derive(Debug)]
pub struct Serving {
    /// `http://127.0.0.1:<port>/mcp`.
    pub url: String,
    /// The bearer token a client must send, when one was asked for.
    pub token: Option<String>,
    /// The address it bound (the port is the real one when `0` was asked).
    pub addr: SocketAddr,
    /// The task serving it; abort it to stop.
    pub task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Serving {
    /// Say where it is: [`crate::publish`] `name` with this URL and token.
    ///
    /// # Errors
    ///
    /// As [`crate::publish`].
    pub fn publish(&self, name: &str) -> std::io::Result<PathBuf> {
        crate::publish(name, &self.url, self.token.as_deref())
    }
}

impl Server {
    /// Serve on loopback (`127.0.0.1`), on the runtime this is awaited on:
    /// the port from the variable `port_env` (any free port when unset),
    /// the token from `token_env` (none when unset or empty). `Ok(None)`
    /// when `FTS_MCP=0` turns MCP off.
    ///
    /// Binds and spawns only; [`Serving::publish`] says where it is.
    ///
    /// # Errors
    ///
    /// `port_env` is set to something that is not a port, or the port
    /// could not be bound (taken).
    pub async fn serve_loopback(
        self,
        port_env: &str,
        token_env: &str,
    ) -> std::io::Result<Option<Serving>> {
        self.serve_loopback_or(0, port_env, token_env).await
    }

    /// [`Self::serve_loopback`] on `default_port` when `port_env` is unset
    /// — a fixed port a project's `.mcp.json` can name.
    ///
    /// # Errors
    ///
    /// As [`Self::serve_loopback`].
    pub async fn serve_loopback_or(
        self,
        default_port: u16,
        port_env: &str,
        token_env: &str,
    ) -> std::io::Result<Option<Serving>> {
        let env = |name: &str| std::env::var(name).ok();
        let Some(plan) = Loopback::from_env(env, default_port, port_env, token_env)? else {
            return Ok(None);
        };
        self.serve_planned(plan).await.map(Some)
    }

    async fn serve_planned(self, plan: Loopback) -> std::io::Result<Serving> {
        let listener =
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, plan.port)).await?;
        let addr = listener.local_addr()?;
        let url = format!("http://{addr}/mcp");
        let task = tokio::spawn(serve(listener, self, plan.token.clone()));
        Ok(Serving {
            url,
            token: plan.token,
            addr,
            task,
        })
    }
}

/// What the environment asks of a loopback server.
#[derive(Debug, PartialEq, Eq)]
struct Loopback {
    port: u16,
    token: Option<String>,
}

impl Loopback {
    /// `None` when `FTS_MCP=0`; otherwise the port (`port_env`, else
    /// `default_port`) and token (`token_env` when non-empty).
    fn from_env(
        env: impl Fn(&str) -> Option<String>,
        default_port: u16,
        port_env: &str,
        token_env: &str,
    ) -> std::io::Result<Option<Self>> {
        if crate::discovery::disabled_by(env(crate::DISABLE_ENV).as_deref()) {
            return Ok(None);
        }
        let port = match env(port_env).filter(|p| !p.trim().is_empty()) {
            None => default_port,
            Some(p) => p.trim().parse::<u16>().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{port_env}={p:?} is not a port"),
                )
            })?,
        };
        let token = env(token_env).filter(|t| !t.is_empty());
        Ok(Some(Self { port, token }))
    }
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
    answer(Arc::clone(&shared.server), ActionCaller::app(), &body).await
}

/// Answer one JSON-RPC message in `body` as `caller`: its reply as JSON,
/// or `202 Accepted` and nothing for a notification.
pub(crate) async fn answer(server: Arc<Server>, caller: ActionCaller, body: &[u8]) -> Response {
    let Ok(text) = String::from_utf8(body.to_vec()) else {
        return (StatusCode::BAD_REQUEST, "not UTF-8").into_response();
    };
    // `watch` may wait for the app: off the async threads.
    // An `app()` caller names no principal, so it runs as the server's
    // own, exactly as `Server::handle` does.
    let handled = tokio::task::spawn_blocking(move || server.handle_with(&caller, &text)).await;
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
    use super::{Loopback, is_local, router};
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

    /// One POST to `/mcp` at `at`, raw, with the bearer token when given.
    async fn post_to(at: std::net::SocketAddr, body: &str, token: Option<&str>) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = tokio::net::TcpStream::connect(at).await.expect("connected");
        let mut request = format!(
            "POST /mcp HTTP/1.1\r\nHost: {at}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(t) = token {
            request.push_str("Authorization: Bearer ");
            request.push_str(t);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        request.push_str(body);
        stream.write_all(request.as_bytes()).await.expect("sent");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.expect("read");
        reply
    }

    /// The host and port of `http://<addr>/mcp`.
    fn addr_of(url: &str) -> std::net::SocketAddr {
        url.strip_prefix("http://")
            .and_then(|rest| rest.strip_suffix("/mcp"))
            .expect("an http://<addr>/mcp url")
            .parse()
            .expect("an address")
    }

    /// Served on loopback, published, found again through the discovery
    /// file alone: `tools/list` over the published URL with the published
    /// token answers the five tools; without the token, refused.
    #[tokio::test]
    async fn tools_list_over_the_published_url_with_the_token() {
        let scratch = crate::discovery::tests::Scratch::new("loopback");
        let serving = Server::new("Test", "0", ActionList::new())
            .serve_planned(Loopback {
                port: 0,
                token: Some("s3cret".into()),
            })
            .await
            .expect("served");
        assert!(serving.addr.ip().is_loopback(), "{}", serving.addr);
        assert_eq!(serving.url, format!("http://{}/mcp", serving.addr));
        crate::publish_in(&scratch.0, "test", &serving.url, serving.token.as_deref())
            .expect("published");

        let found = crate::read_in(&scratch.0, "test").expect("found");
        let at = addr_of(&found.url);
        let list = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#;
        let refused = post_to(at, list, None).await;
        assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
        let reply = post_to(at, list, found.token.as_deref()).await;
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        let body = reply.split("\r\n\r\n").nth(1).expect("a body");
        let value: serde_json::Value = serde_json::from_str(body).expect("json");
        let names: Vec<&str> = value["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(names, ["actions", "describe", "run", "batch", "watch"]);
        serving.task.abort();
    }

    /// The public door: unset variables mean any free port and no token.
    #[tokio::test]
    async fn serve_loopback_with_nothing_set() {
        let served = Server::new("Test", "0", ActionList::new())
            .serve_loopback(
                "ARCHITECT_MCP_TEST_UNSET_PORT_STA385",
                "ARCHITECT_MCP_TEST_UNSET_TOKEN_STA385",
            )
            .await
            .expect("bound");
        if crate::disabled() {
            assert!(served.is_none());
            return;
        }
        let serving = served.expect("on");
        assert!(serving.token.is_none());
        assert_ne!(serving.addr.port(), 0);
        let reply = post_to(
            serving.addr,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            None,
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        serving.task.abort();
    }

    #[test]
    fn the_environment_plans_the_server() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let plan = |pairs| Loopback::from_env(env(pairs), 47_811, "P", "T");
        assert_eq!(
            plan(&[]).expect("ok"),
            Some(Loopback {
                port: 47_811,
                token: None
            })
        );
        assert_eq!(
            plan(&[("P", "9"), ("T", "tok")]).expect("ok"),
            Some(Loopback {
                port: 9,
                token: Some("tok".into())
            })
        );
        assert_eq!(
            plan(&[("T", "")]).expect("ok"),
            Some(Loopback {
                port: 47_811,
                token: None
            })
        );
        assert_eq!(plan(&[("FTS_MCP", "0"), ("P", "9")]).expect("ok"), None);
        assert!(plan(&[("FTS_MCP", "1")]).expect("ok").is_some());
        assert_eq!(
            plan(&[("P", "http")]).expect_err("not a port").kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
