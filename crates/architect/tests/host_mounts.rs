//! `EngineHost` protocol mounts: `vox_at` resolves each upgrade from its
//! path params and bearer, `dav` answers every verb outside the `finish`
//! layers, and `document` serves a `GET` the SPA fallback never answers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use architect::LayerRouter;
use architect::host::{DavRequest, EngineHost, RequestHead};
use architect::http::axum::Router;
use architect::http::axum::body::{Body, to_bytes};
use architect::http::axum::http::{Method, Request, StatusCode};
use architect::http::axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tower::ServiceExt as _;

/// A resolver for `/org/{slug}/vox`: only `acme` is hosted, and it wants
/// a bearer.
async fn resolve_org(head: RequestHead) -> Result<LayerRouter, (StatusCode, &'static str)> {
    if head.param("slug") != Some("acme") {
        return Err((StatusCode::NOT_FOUND, "org not hosted"));
    }
    match head.bearer() {
        Some("t0ken") => Ok(LayerRouter::new()),
        _ => Err((StatusCode::UNAUTHORIZED, "who are you")),
    }
}

/// The status line and headers of a raw WebSocket upgrade to `path`.
async fn upgrade(at: std::net::SocketAddr, path: &str, extra: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(at).await.unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {at}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{extra}\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = vec![0_u8; 1024];
    let n = stream.read(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf[..n]).to_lowercase()
}

#[tokio::test]
async fn vox_at_resolves_each_upgrade_from_its_params_and_bearer() {
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .vox_at("/org/{slug}/vox", resolve_org)
        .into_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await });

    let browser = "Sec-WebSocket-Protocol: vox.v1, vox.bearer.t0ken\r\n";
    let ok = upgrade(at, "/org/acme/vox", browser).await;
    assert!(ok.starts_with("http/1.1 101"), "{ok}");
    assert!(ok.contains("sec-websocket-protocol: vox.v1"), "{ok}");

    let native = "Sec-WebSocket-Protocol: vox.v1\r\nAuthorization: Bearer t0ken\r\n";
    let ok = upgrade(at, "/org/acme/vox", native).await;
    assert!(ok.starts_with("http/1.1 101"), "{ok}");

    let anonymous = upgrade(at, "/org/acme/vox", "Sec-WebSocket-Protocol: vox.v1\r\n").await;
    assert!(anonymous.starts_with("http/1.1 401"), "{anonymous}");
    let elsewhere = upgrade(at, "/org/other/vox", browser).await;
    assert!(elsewhere.starts_with("http/1.1 404"), "{elsewhere}");

    // The built-in `/vox` is still there beside it.
    let plain = upgrade(at, "/vox", "Sec-WebSocket-Protocol: vox.v1\r\n").await;
    assert!(plain.starts_with("http/1.1 101"), "{plain}");
}

#[tokio::test]
async fn vox_at_the_root_replaces_the_built_in_vox() {
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .vox_at("/vox", |head: RequestHead| async move {
            match head.bearer() {
                Some(_) => Ok(LayerRouter::new()),
                None => Err(StatusCode::UNAUTHORIZED),
            }
        })
        .into_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await });

    let anonymous = upgrade(at, "/vox", "Sec-WebSocket-Protocol: vox.v1\r\n").await;
    assert!(anonymous.starts_with("http/1.1 401"), "{anonymous}");
    let signed_in = "Sec-WebSocket-Protocol: vox.v1, vox.bearer.any\r\n";
    let ok = upgrade(at, "/vox", signed_in).await;
    assert!(ok.starts_with("http/1.1 101"), "{ok}");
}

/// A `finish` layer that answers every `OPTIONS` itself, as tower-http's
/// CORS layer does.
fn cors_like(app: Router) -> Router {
    app.layer(architect::http::axum::middleware::from_fn(
        |req: Request<Body>, next: architect::http::axum::middleware::Next| async move {
            if req.method() == Method::OPTIONS {
                return "cors".into_response();
            }
            next.run(req).await
        },
    ))
}

/// Echo what the handler was given: `method mount rel slug`.
async fn echo_dav(dav: DavRequest) -> Response {
    let slug = dav.head.param("slug").unwrap_or("-").to_owned();
    let line = format!(
        "{} {} {} {slug} {}",
        dav.request.method(),
        dav.mount,
        if dav.rel.is_empty() { "-" } else { &dav.rel },
        dav.head.params().len()
    );
    ([("dav", "1")], line).into_response()
}

async fn send(app: Router, method: &str, path: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn dav_answers_every_verb_outside_the_finish_layers() {
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .dav("/org/{slug}/dav", echo_dav)
        .finish(cors_like)
        .into_app();

    // A file manager's first request: a bare OPTIONS on the collection.
    let (status, body) = send(app.clone(), "OPTIONS", "/org/acme/dav/").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, "OPTIONS /org/acme/dav - acme 1")
    );
    let (_, body) = send(app.clone(), "PROPFIND", "/org/acme/dav").await;
    assert_eq!(body, "PROPFIND /org/acme/dav - acme 1");
    let (_, body) = send(app.clone(), "MKCOL", "/org/acme/dav/Songs/new%20take").await;
    assert_eq!(
        body, "MKCOL /org/acme/dav Songs/new take acme 1",
        "the in-mount path is decoded and not a param"
    );
    // Everything else is still behind the layer.
    let (_, body) = send(app, "OPTIONS", "/health").await;
    assert_eq!(body, "cors");
}

#[tokio::test]
async fn document_answers_get_with_the_head_and_beats_the_spa() {
    let dir = std::env::temp_dir().join(format!("architect-host-mounts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "spa").unwrap();
    let app = EngineHost::new(LayerRouter::new(), "127.0.0.1:0")
        .web(Some(architect::host::WebBundle::Dir(dir.clone())))
        .document("/.well-known/app.json", |head: RequestHead| async move {
            Ok::<_, StatusCode>(format!("{{\"signed_in\":{}}}", head.bearer().is_some()))
        })
        .document("/org/{slug}/api", |head: RequestHead| async move {
            match head.param("slug") {
                Some("acme") => Ok("acme api"),
                _ => Err(StatusCode::NOT_FOUND),
            }
        })
        .into_app();

    let signed_in = Request::get("/.well-known/app.json")
        .header("authorization", "Bearer x")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(signed_in).await.unwrap();
    let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    assert_eq!(&body[..], br#"{"signed_in":true}"#);

    assert_eq!(
        send(app.clone(), "GET", "/.well-known/app.json").await,
        (StatusCode::OK, r#"{"signed_in":false}"#.to_owned())
    );
    assert_eq!(
        send(app.clone(), "GET", "/org/acme/api").await,
        (StatusCode::OK, "acme api".to_owned())
    );
    let (status, body) = send(app.clone(), "GET", "/org/other/api").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        send(app, "GET", "/somewhere/else").await,
        (StatusCode::OK, "spa".to_owned())
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
