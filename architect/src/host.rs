//! `architect::host` — build a headless vox engine binary from a router.
//!
//! Every engine binary repeats the same bootstrap: a multi-thread tokio
//! runtime, a tracing subscriber, a backtrace panic hook, then an axum app
//! with `/health` + `/vox` (the router), optionally the same router over iroh
//! p2p, and optionally a static SPA bundle as the HTTP fallback. This module
//! owns all of it; a binary supplies only its router (business logic) and
//! calls [`EngineHost::serve`].
//!
//! Native only (the `host` feature); the wasm client build never sees it.

use std::future::Future;
use std::path::PathBuf;

use axum::extract::WebSocketUpgrade;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::LayerRouter;

/// Build a multi-thread tokio runtime and block on `fut` until it completes —
/// the entry point of an engine binary (`fn run() { host::block_on(main()) }`).
pub fn block_on<F: Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        // 16 MiB worker stacks (reserved, not committed): vox 0.10's
        // debug-build channel encode recurses deeply on large payloads
        // (e.g. session Setlists served over `/vox`) and overflows tokio's
        // default 2 MiB workers. Engine binaries serve those payloads on
        // this runtime, so size it like the in-process session engines do.
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("tokio runtime")
        .block_on(fut)
}

/// Initialize the global tracing subscriber from `RUST_LOG`, falling back to
/// `default_filter` (e.g. `"info"`).
pub fn init_tracing(default_filter: &str) {
    let default = default_filter.to_string();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| default.into()),
        )
        .init();
}

/// Install a panic hook that logs the panicking thread + a backtrace via
/// `tracing` before the previous hook runs. Panics still unwind — this only
/// guarantees none dies silently mid-service.
pub fn install_panic_logger() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            thread = thread.name().unwrap_or("<unnamed>"),
            "panic: {info}\n{backtrace}"
        );
        default_hook(info);
    }));
}

/// A resolved static web (SPA) bundle to serve as the app's fallback route.
pub enum WebBundle {
    /// A directory on disk; served with an `index.html` SPA fallback.
    Dir(PathBuf),
    /// Assets embedded in the binary via `include_dir!` (created by the app,
    /// which owns the manifest path).
    Embedded(&'static include_dir::Dir<'static>),
}

/// A headless engine host. Serves a vox [`LayerRouter`] over axum with a
/// `/vox` WebSocket + a `/health` route, optionally over iroh p2p, and
/// optionally with a static SPA bundle as the HTTP fallback.
pub struct EngineHost {
    router: LayerRouter,
    addr: String,
    web: Option<WebBundle>,
    extra: Option<Router>,
    #[cfg(feature = "iroh")]
    iroh: Option<IrohConfig>,
}

#[cfg(feature = "iroh")]
struct IrohConfig {
    key_path: PathBuf,
    id_path: Option<PathBuf>,
}

impl EngineHost {
    /// A host serving `router` on `addr` (e.g. `"0.0.0.0:4040"`).
    pub fn new(router: LayerRouter, addr: impl Into<String>) -> Self {
        Self {
            router,
            addr: addr.into(),
            web: None,
            extra: None,
            #[cfg(feature = "iroh")]
            iroh: None,
        }
    }

    /// Merge extra axum routes onto the host app (e.g. an HTTP bridge for
    /// clients that can't speak vox over WebSocket — watchOS remotes). The
    /// routes are mounted alongside `/health` + `/vox`, before the SPA
    /// fallback, so they win over the web bundle.
    pub fn extend(mut self, routes: Router) -> Self {
        self.extra = Some(match self.extra {
            Some(existing) => existing.merge(routes),
            None => routes,
        });
        self
    }

    /// Also serve the router over an iroh endpoint. The secret key persists at
    /// `key_path` (stable id across restarts); the endpoint id is written to
    /// `id_path` when given, for other devices/agents to read.
    #[cfg(feature = "iroh")]
    pub fn iroh(mut self, key_path: PathBuf, id_path: Option<PathBuf>) -> Self {
        self.iroh = Some(IrohConfig { key_path, id_path });
        self
    }

    /// Serve `bundle` as the HTTP fallback (the browser remote). `None` leaves
    /// the host headless (only `/health` + `/vox`).
    pub fn web(mut self, bundle: Option<WebBundle>) -> Self {
        self.web = bundle;
        self
    }

    /// Bind and serve until the server dies. Never returns on success.
    pub async fn serve(self) {
        let router = self.router;

        #[cfg(feature = "iroh")]
        if let Some(cfg) = self.iroh {
            tokio::spawn(serve_iroh(router.clone(), cfg));
        }

        let vox_router = router;
        let mut app = Router::new().route("/health", get(|| async { "ok" })).route(
            "/vox",
            get(move |ws: WebSocketUpgrade| {
                let router = vox_router.clone();
                async move {
                    ws.on_upgrade(move |socket| crate::axum_ws::serve_router(socket, router))
                        .into_response()
                }
            }),
        );

        if let Some(extra) = self.extra {
            app = app.merge(extra);
        }

        match self.web {
            Some(WebBundle::Dir(dir)) => {
                use tower_http::services::{ServeDir, ServeFile};
                let index = dir.join("index.html");
                app = app.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(index)));
                tracing::info!("web remote fallback: dir {}", dir.display());
            }
            Some(WebBundle::Embedded(dir)) => {
                app = app.fallback(get(move |uri: axum::http::Uri| async move {
                    embedded_asset(dir, uri)
                }));
                tracing::info!("web remote fallback: embedded bundle");
            }
            None => {
                tracing::warn!("no web bundle — serving /health + /vox only");
            }
        }

        let listener = tokio::net::TcpListener::bind(&self.addr)
            .await
            .unwrap_or_else(|e| panic!("bind {}: {e}", self.addr));
        tracing::info!("engine serving ws://{}/vox", self.addr);
        axum::serve(listener, app).await.expect("axum serve");
    }
}

#[cfg(feature = "iroh")]
async fn serve_iroh(router: LayerRouter, cfg: IrohConfig) {
    use crate::iroh_link;
    let secret_key = match iroh_link::load_or_create_secret_key(&cfg.key_path) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(error = %e, "iroh secret key unavailable; p2p transport disabled");
            return;
        }
    };
    let endpoint = match iroh_link::bind_endpoint(secret_key).await {
        Ok(ep) => ep,
        Err(e) => {
            tracing::error!(error = %e, "iroh endpoint bind failed; p2p transport disabled");
            return;
        }
    };
    tracing::info!("iroh endpoint id: {}", endpoint.id());
    if let Some(id_path) = &cfg.id_path
        && let Err(e) = std::fs::write(id_path, format!("{}\n", endpoint.id()))
    {
        tracing::warn!(error = %e, "could not write iroh endpoint-id file");
    }
    iroh_link::serve_router(&endpoint, router).await;
}

/// Serve an embedded SPA bundle from memory: an exact file match, else
/// `index.html` (client-side routing). Content type is inferred from the path.
fn embedded_asset(dir: &'static include_dir::Dir<'static>, uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (path, file) = match dir.get_file(path) {
        Some(f) if !path.is_empty() => (path, f),
        _ => match dir.get_file("index.html") {
            Some(f) => ("index.html", f),
            None => {
                return (axum::http::StatusCode::NOT_FOUND, "no index.html in embedded bundle")
                    .into_response();
            }
        },
    };
    (
        [(axum::http::header::CONTENT_TYPE, content_type_for(path))],
        file.contents(),
    )
        .into_response()
}

/// Content type by file extension — enough for a dx web bundle without a mime
/// crate.
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
