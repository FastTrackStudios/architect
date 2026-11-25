//! The server's host: one axum app serving `/health`, vox at `/vox` (a
//! WebSocket answered by the services' [`VoxRouter`]), and every route
//! handed to it ([`Host::plugin`]); and what the binary does first
//! ([`boot`]).

use architect_auth::vox_host::{VoxRouter, axum_ws};
use axum::Router;
use axum::extract::WebSocketUpgrade;
use axum::routing::get;

pub use architect_auth::vox_host::VOX_SUBPROTOCOL;

type Wrap = Box<dyn FnOnce(Router) -> Router + Send>;

/// Everything the server answers, not yet listening.
pub struct Host {
    vox: VoxRouter,
    routes: Router,
    addr: String,
    wraps: Vec<Wrap>,
}

impl Host {
    /// The services' vox router, to be served on `addr` (`0.0.0.0:8080`).
    #[must_use]
    pub fn new(vox: VoxRouter, addr: impl Into<String>) -> Self {
        Self {
            vox,
            routes: Router::new(),
            addr: addr.into(),
            wraps: Vec::new(),
        }
    }

    /// More routes beside `/vox`.
    #[must_use]
    pub fn plugin(mut self, routes: Router) -> Self {
        self.routes = self.routes.merge(routes);
        self
    }

    /// Wrap the finished app (CORS, tracing), in the order given.
    #[must_use]
    pub fn finish(mut self, wrap: impl FnOnce(Router) -> Router + Send + 'static) -> Self {
        self.wraps.push(Box::new(wrap));
        self
    }

    /// The axum app, for in-process use (tests) or to serve.
    #[must_use]
    pub fn into_app(self) -> Router {
        let vox = self.vox;
        let mut app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/vox",
                get(move |ws: WebSocketUpgrade| {
                    std::future::ready(axum_ws::upgrade(ws, vox.clone()))
                }),
            )
            .merge(self.routes);
        for wrap in self.wraps {
            app = wrap(app);
        }
        app
    }

    /// Bind and serve until the process ends. A port already in use is an
    /// error to report, not a panic.
    pub async fn serve(self) -> std::io::Result<()> {
        let addr = self.addr.clone();
        let app = self.into_app();
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("auth server serving http://{addr} (vox at /vox)");
        axum::serve(listener, app).await
    }
}

/// Fields of the request's own span (its wide event): recorded on the
/// current span when it declares the field, and nothing otherwise.
pub mod wide {
    pub fn set(key: &'static str, value: &str) {
        tracing::Span::current().record(key, value);
    }

    /// [`set`] for any `Display` value.
    pub fn set_display(key: &'static str, value: impl core::fmt::Display) {
        set(key, &value.to_string());
    }
}

/// Log targets whose DEBUG and TRACE records write credentials (a
/// WebSocket's frames carry bearer tokens).
pub const CREDENTIAL_LOGGING_TARGETS: &[&str] = &["tungstenite", "tokio_tungstenite"];

/// Whether a record is one the log never lets through: DEBUG or TRACE
/// from one of [`CREDENTIAL_LOGGING_TARGETS`] or a module of one.
#[must_use]
pub fn writes_credentials(meta: &tracing::Metadata<'_>) -> bool {
    blocked(*meta.level(), meta.target())
}

fn blocked(level: tracing::Level, target: &str) -> bool {
    level > tracing::Level::INFO
        && CREDENTIAL_LOGGING_TARGETS.iter().any(|t| {
            target
                .strip_prefix(t)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
        })
}

/// What the binary does first: logs from `RUST_LOG` (or `default_filter`),
/// never a record that [`writes_credentials`] whatever `RUST_LOG` says,
/// and a panic hook that logs the panic before the default one runs.
pub fn boot(default_filter: &str) {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::filter::FilterExt as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.to_string().into());
    let capped = filter.and(tracing_subscriber::filter::filter_fn(|meta| {
        !writes_credentials(meta)
    }));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(capped))
        .init();

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

#[cfg(test)]
mod tests {
    use super::blocked;
    use tracing::Level;

    #[test]
    fn a_websockets_debug_records_never_reach_the_log() {
        assert!(blocked(Level::DEBUG, "tungstenite::protocol"));
        assert!(blocked(Level::TRACE, "tokio_tungstenite"));
        assert!(!blocked(Level::INFO, "tungstenite::protocol"));
        assert!(!blocked(Level::DEBUG, "tungstenite_lookalike"));
        assert!(!blocked(Level::DEBUG, "auth_server"));
    }
}
