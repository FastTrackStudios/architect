//! `architect::host` — build a headless vox engine binary from a router.
//!
//! Every engine binary repeats the same bootstrap: a multi-thread tokio
//! runtime, a tracing subscriber, a backtrace panic hook, then an axum app
//! with `/health` + `/vox` (the router), optionally the same router over iroh
//! p2p, and optionally a static SPA bundle as the HTTP fallback. This module
//! owns all of it; a binary supplies only its router (business logic) and
//! calls [`EngineHost::serve`].
//!
//! Every host also answers `GET /version.json` with the binary's
//! [`BuildInfo`] (`{"rev", "version", "built_at"}`), mounted before the SPA
//! fallback so the bundle can never answer it. A binary fills it from its
//! own build-time env with [`build_info!`](crate::build_info).
//!
//! Native only (the `host` feature); the wasm client build never sees it.

use std::future::Future;
use std::path::PathBuf;

use axum::Router;
use axum::extract::WebSocketUpgrade;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::LayerRouter;

mod mount;
pub use mount::{BEARER_SUBPROTOCOL_PREFIX, DavRequest, RequestHead};

/// The vox WebSocket subprotocol `/vox` selects.
///
/// Every client offers it, and a browser client that offers a subprotocol
/// gets no connection at all unless the server echoes it — so this is not
/// cosmetic.
pub const VOX_SUBPROTOCOL: &str = "vox.v1";

/// Build a multi-thread tokio runtime and block on `fut` until it completes.
///
/// The entry point of an engine binary
/// (`fn run() { host::block_on(main()) }`).
///
/// # Panics
///
/// If the tokio runtime cannot be built — the OS refused to spawn worker
/// threads or allocate their stacks. There is no engine to run without
/// it, and no caller that could do anything with the error, so this one
/// stays a panic and says so.
#[allow(clippy::expect_used)]
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
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default.into()),
        )
        .init();
}

/// The two calls every binary makes first: tracing from `RUST_LOG` (or
/// `default_filter`) and the panic logger.
pub fn boot(default_filter: &str) {
    init_tracing(default_filter);
    install_panic_logger();
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

/// The build-time env [`build_info!`](crate::build_info) reads for the
/// commit a binary was built from (e.g. `git rev-parse HEAD`).
pub const BUILD_REV_ENV: &str = "ARCHITECT_BUILD_REV";

/// The build-time env [`build_info!`](crate::build_info) reads for when a
/// binary was built (RFC 3339 UTC, e.g. `date -u +%Y-%m-%dT%H:%M:%SZ`).
pub const BUILT_AT_ENV: &str = "ARCHITECT_BUILT_AT";

/// What `GET /version.json` answers: which build of the binary is running.
///
/// A field the build did not set is `null` in the JSON, never a guess.
/// Build one with [`build_info!`](crate::build_info), which reads the
/// calling crate's version and the [`BUILD_REV_ENV`] / [`BUILT_AT_ENV`]
/// variables at compile time, and hand it to [`EngineHost::build_info`].
#[derive(Debug, Clone, PartialEq, Eq, Default, facet::Facet)]
pub struct BuildInfo {
    /// The commit the binary was built from.
    pub rev: Option<String>,
    /// The binary crate's version (`CARGO_PKG_VERSION`).
    pub version: Option<String>,
    /// When the binary was built, RFC 3339 UTC.
    pub built_at: Option<String>,
}

impl BuildInfo {
    /// A `BuildInfo` from the three values; empty strings count as unset,
    /// so a CI step that exports an empty variable still serves `null`.
    #[must_use]
    pub fn new(version: Option<&str>, rev: Option<&str>, built_at: Option<&str>) -> Self {
        let set = |v: Option<&str>| v.filter(|s| !s.is_empty()).map(str::to_owned);
        Self {
            rev: set(rev),
            version: set(version),
            built_at: set(built_at),
        }
    }

    /// The JSON `/version.json` serves.
    ///
    /// # Errors
    ///
    /// If facet-json cannot encode the struct (it has only optional
    /// strings, so this is not expected).
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }
}

/// The calling crate's [`BuildInfo`](crate::host::BuildInfo), read at
/// compile time: `version` from its `CARGO_PKG_VERSION`, `rev` from
/// `ARCHITECT_BUILD_REV` and `built_at` from `ARCHITECT_BUILT_AT`.
///
/// Call it in the binary crate, not a library, so the version is the
/// binary's. Cargo tracks both variables, so changing either rebuilds the
/// crate that expands the macro.
///
/// ```ignore
/// EngineHost::new(router, bind).build_info(architect::build_info!())
/// ```
#[macro_export]
macro_rules! build_info {
    () => {
        $crate::host::BuildInfo::new(
            ::core::option_env!("CARGO_PKG_VERSION"),
            ::core::option_env!("ARCHITECT_BUILD_REV"),
            ::core::option_env!("ARCHITECT_BUILT_AT"),
        )
    };
}

/// A resolved static web (SPA) bundle to serve as the app's fallback route.
pub enum WebBundle {
    /// A directory on disk; served with an `index.html` SPA fallback.
    Dir(PathBuf),
    /// Assets embedded in the binary via `include_dir!` (created by the app,
    /// which owns the manifest path).
    Embedded(&'static include_dir::Dir<'static>),
}

/// A headless engine host.
///
/// Serves a vox [`LayerRouter`] over axum with a `/vox` WebSocket +
/// `/health` + `/version.json` routes, optionally over iroh p2p, and
/// optionally with a static SPA bundle as the HTTP fallback.
pub struct EngineHost {
    router: LayerRouter,
    addr: String,
    web: Option<WebBundle>,
    build: BuildInfo,
    extra: Option<Router>,
    /// Routes merged after the `finish` layers (WebDAV): see [`EngineHost::dav`].
    outside: Option<Router>,
    /// `vox_at("/vox", …)` replaced the built-in `/vox`.
    root_vox_resolved: bool,
    finish: Vec<Box<dyn FnOnce(Router) -> Router + Send>>,
    cross_origin_isolated: bool,
    #[cfg(feature = "iroh")]
    iroh: Option<IrohConfig>,
    #[cfg(feature = "iroh")]
    iroh_lookup: crate::iroh_link::PeerLookup,
}

#[cfg(feature = "iroh")]
struct IrohConfig {
    key_path: PathBuf,
    id_path: Option<PathBuf>,
    lookup: crate::iroh_link::PeerLookup,
}

impl EngineHost {
    /// A host serving `router` on `addr` (e.g. `"0.0.0.0:4040"`).
    pub fn new(router: LayerRouter, addr: impl Into<String>) -> Self {
        Self {
            router,
            addr: addr.into(),
            cross_origin_isolated: false,
            web: None,
            build: BuildInfo::default(),
            extra: None,
            outside: None,
            root_vox_resolved: false,
            finish: Vec::new(),
            #[cfg(feature = "iroh")]
            iroh: None,
            #[cfg(feature = "iroh")]
            iroh_lookup: crate::iroh_link::PeerLookup::default(),
        }
    }

    /// What `GET /version.json` answers; pass
    /// [`build_info!()`](crate::build_info) from the binary crate. Without
    /// it the route still answers, with every field `null`.
    #[must_use]
    pub fn build_info(mut self, info: BuildInfo) -> Self {
        self.build = info;
        self
    }

    /// Mount a plugin: any axum router — a generated HTTP face
    /// (`layers![…].provide_http(&backend)`), hosted pages, an OAuth
    /// flow. Alias of [`extend`](Self::extend) that reads as what it is
    /// at the call site.
    ///
    /// The escape hatch. A capability is a service trait mounted through
    /// `provide_http`; a protocol endpoint has a builder method
    /// ([`vox_at`](Self::vox_at), [`dav`](Self::dav),
    /// [`document`](Self::document)). Reach for a raw router only for
    /// what neither covers yet, so `cargo xtask boundaries` (`hand-routes`)
    /// keeps counting it.
    #[must_use]
    pub fn plugin(self, routes: Router) -> Self {
        self.extend(routes)
    }

    /// Wrap the assembled app once everything is mounted — the place for
    /// outermost tower layers (CORS, tracing, timeouts) that must see
    /// every route, including the SPA fallback. Applied in call order.
    #[must_use]
    pub fn finish(mut self, wrap: impl FnOnce(Router) -> Router + Send + 'static) -> Self {
        self.finish.push(Box::new(wrap));
        self
    }

    /// Merge extra axum routes onto the host app (e.g. an HTTP bridge for
    /// clients that can't speak vox over WebSocket — watchOS remotes). The
    /// routes are mounted alongside `/health` + `/vox`, before the SPA
    /// fallback, so they win over the web bundle.
    #[must_use]
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
    #[must_use]
    pub fn iroh(mut self, key_path: PathBuf, id_path: Option<PathBuf>) -> Self {
        self.iroh = Some(IrohConfig {
            key_path,
            id_path,
            // Replaced by `iroh_lookup` in `assemble`, so call order
            // between the two builder methods doesn't matter.
            lookup: crate::iroh_link::PeerLookup::default(),
        });
        self
    }

    /// How the iroh endpoint finds peers and publishes itself (default
    /// [`PeerLookup::N0`](crate::iroh_link::PeerLookup::N0)). With the
    /// `iroh-lan` feature, `PeerLookup::N0AndLan` keeps a host dialable
    /// by bare id from the same LAN when the internet is down. Only takes
    /// effect together with [`iroh`](Self::iroh), in either order.
    #[cfg(feature = "iroh")]
    #[must_use]
    pub const fn iroh_lookup(mut self, lookup: crate::iroh_link::PeerLookup) -> Self {
        self.iroh_lookup = lookup;
        self
    }

    /// Serve `bundle` as the HTTP fallback (the browser remote). `None` leaves
    /// the host headless (only `/health` + `/vox`).
    #[must_use]
    pub fn web(mut self, bundle: Option<WebBundle>) -> Self {
        self.web = bundle;
        self
    }

    /// Serve every response with the two headers that make the page
    /// **cross-origin isolated**:
    ///
    /// ```text
    /// Cross-Origin-Opener-Policy:   same-origin
    /// Cross-Origin-Embedder-Policy: require-corp
    /// ```
    ///
    /// That is the browser's precondition for `SharedArrayBuffer`, and so
    /// for shared wasm memory and wasm threads — an audio engine cannot
    /// have a real streamer thread pool in the browser without it.
    ///
    /// The cost is that EVERY subresource must be same-origin or carry
    /// CORP/CORS. Turn it on only for a host whose assets it owns (the
    /// engine serves its own bundle, so it qualifies); a host embedding
    /// third-party iframes or CDN assets must not.
    #[must_use]
    pub const fn cross_origin_isolated(mut self, on: bool) -> Self {
        self.cross_origin_isolated = on;
        self
    }

    /// The assembled axum app — `/health` + `/version.json` + `/vox` + every plugin + the
    /// SPA fallback + the `finish` layers — without binding a listener.
    /// What an in-process test drives with `tower::ServiceExt::oneshot`,
    /// and what [`serve`](Self::serve) binds. The iroh side, if
    /// configured, is not started here: it needs a running server task.
    #[must_use]
    pub fn into_app(self) -> Router {
        let (app, _iroh) = self.assemble();
        app
    }

    #[cfg(feature = "iroh")]
    fn assemble(self) -> (Router, Option<(LayerRouter, IrohConfig)>) {
        let lookup = self.iroh_lookup;
        let iroh = self
            .iroh
            .map(|cfg| (self.router.clone(), IrohConfig { lookup, ..cfg }));
        (
            assemble_app(
                (!self.root_vox_resolved).then_some(self.router),
                self.web,
                self.build,
                self.extra,
                self.outside,
                self.finish,
                self.cross_origin_isolated,
            ),
            iroh,
        )
    }

    #[cfg(not(feature = "iroh"))]
    fn assemble(self) -> (Router, Option<()>) {
        (
            assemble_app(
                (!self.root_vox_resolved).then_some(self.router),
                self.web,
                self.build,
                self.extra,
                self.outside,
                self.finish,
                self.cross_origin_isolated,
            ),
            None,
        )
    }

    /// Bind and serve until the server dies. Never returns on success.
    pub async fn serve(self) -> std::io::Result<()> {
        let addr = self.addr.clone();
        let (app, iroh) = self.assemble();

        #[cfg(feature = "iroh")]
        if let Some((router, cfg)) = iroh {
            tokio::spawn(serve_iroh(router, cfg));
        }
        #[cfg(not(feature = "iroh"))]
        let _ = iroh;

        // A port already in use is an ordinary operational condition —
        // another engine is running, or the port is privileged. Returning
        // it lets the binary print something useful and pick an exit code,
        // instead of dying with a panic backtrace in the middle of startup.
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("engine serving ws://{addr}/vox");
        axum::serve(listener, app).await
    }
}

fn assemble_app(
    router: Option<LayerRouter>,
    web: Option<WebBundle>,
    build: BuildInfo,
    extra: Option<Router>,
    outside: Option<Router>,
    finish: Vec<Box<dyn FnOnce(Router) -> Router + Send>>,
    cross_origin_isolated: bool,
) -> Router {
    let mut app = Router::new()
        .route("/health", get(|| async { "ok" }))
        // A real route, not a fallback, so a SPA bundle never answers it.
        .route(
            "/version.json",
            get(move || std::future::ready(version_json(&build))),
        );
    // `None` when `vox_at("/vox", …)` resolves `/vox` per connection instead.
    if let Some(vox_router) = router {
        app = app.route(
            "/vox",
            get(move |ws: WebSocketUpgrade| {
                let router = vox_router.clone();
                async move {
                    ws.protocols([VOX_SUBPROTOCOL])
                        .on_upgrade(move |socket| crate::axum_ws::serve_router(socket, router))
                        .into_response()
                }
            }),
        );
    }

    if let Some(extra) = extra {
        app = app.merge(extra);
    }

    match web {
        Some(WebBundle::Dir(dir)) => {
            use tower_http::services::{ServeDir, ServeFile};
            let index = dir.join("index.html");
            app = app.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(index)));
            tracing::info!("web remote fallback: dir {}", dir.display());
        }
        Some(WebBundle::Embedded(dir)) => {
            app = app.fallback(get(move |uri: axum::http::Uri| async move {
                embedded_asset(dir, &uri)
            }));
            tracing::info!("web remote fallback: embedded bundle");
        }
        None => {
            tracing::debug!("no web bundle — serving /health + /vox + plugins");
        }
    }

    if cross_origin_isolated {
        use axum::http::{HeaderName, HeaderValue};
        use tower_http::set_header::SetResponseHeaderLayer;
        // `http` has no constants for these three, so name them here.
        const COOP: HeaderName = HeaderName::from_static("cross-origin-opener-policy");
        const COEP: HeaderName = HeaderName::from_static("cross-origin-embedder-policy");
        const CORP: HeaderName = HeaderName::from_static("cross-origin-resource-policy");
        // `overriding` (not `if_not_present`): isolation is all-or-
        // nothing — one response without the pair and the whole page
        // loses `crossOriginIsolated`, taking shared memory with it.
        // CORP same-origin rides along so the bundle's own assets stay
        // loadable under require-corp.
        app = app
            .layer(SetResponseHeaderLayer::overriding(
                COOP,
                HeaderValue::from_static("same-origin"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                COEP,
                HeaderValue::from_static("require-corp"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                CORP,
                HeaderValue::from_static("same-origin"),
            ));
        tracing::info!("cross-origin isolated (COOP/COEP): SharedArrayBuffer enabled");
    }

    for wrap in finish {
        app = wrap(app);
    }
    // After the layers on purpose: `dav` routes must not answer through
    // CORS (see `EngineHost::dav`).
    if let Some(outside) = outside {
        app = app.merge(outside);
    }
    app
}

/// The `/version.json` response: the encoded [`BuildInfo`], never cached
/// (a deploy changes it under the same URL).
fn version_json(build: &BuildInfo) -> Response {
    use axum::http::{StatusCode, header};
    match build.to_json() {
        Ok(body) => (
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            body,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("encode /version.json: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
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
    let endpoint = match iroh_link::bind_endpoint_with(secret_key, cfg.lookup).await {
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
fn embedded_asset(dir: &'static include_dir::Dir<'static>, uri: &axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (path, file) = match dir.get_file(path) {
        Some(f) if !path.is_empty() => (path, f),
        _ => match dir.get_file("index.html") {
            Some(f) => ("index.html", f),
            None => {
                return (
                    axum::http::StatusCode::NOT_FOUND,
                    "no index.html in embedded bundle",
                )
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
