//! Assembling the running server: storage, engine, routers, listener.

use architect::LayerRouter;
use architect_auth::{
    ArchitectAuth, AuthServiceDispatcher,
    db::{AuthSeaOrmStorage, Migrator},
    transport::{
        AuthCookieConfig,
        vox::{AuthServerMiddleware, AuthVoxService},
    },
};
use axum::{
    Router,
    extract::ws::WebSocketUpgrade,
    response::IntoResponse,
    routing::get,
};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::config::ServerConfig;
use crate::http::{self, HttpState};

/// The vox WebSocket subprotocol. A browser client that offers a
/// subprotocol gets no connection at all unless the server echoes it
/// back, so this is not cosmetic.
pub const VOX_SUBPROTOCOL: &str = "vox.v1";

/// A built, not-yet-listening server.
pub struct AuthServer {
    pub auth: ArchitectAuth<AuthSeaOrmStorage>,
    pub app: Router,
    pub bind_addr: String,
    /// Kept so callers (tests, an embedding binary) can reach the same
    /// pool the engine writes through.
    pub db: DatabaseConnection,
}

/// Connect, migrate, and assemble everything from a [`ServerConfig`].
pub async fn build(config: &ServerConfig) -> eyre::Result<AuthServer> {
    let db = Database::connect(&config.database_url)
        .await
        .map_err(|error| eyre::eyre!("connect auth database: {error}"))?;

    if config.run_migrations {
        tracing::info!("running auth migrations");
        Migrator::up(&db, None)
            .await
            .map_err(|error| eyre::eyre!("auth migrations: {error}"))?;
    }

    let auth = build_engine(config, AuthSeaOrmStorage::new(db.clone()))?;
    let app = app_router(config, auth.clone());

    Ok(AuthServer {
        auth,
        app,
        bind_addr: config.bind_addr.clone(),
        db,
    })
}

/// Turn a [`ServerConfig`] into a configured engine.
///
/// Generic over storage so tests can build the same engine over
/// in-memory SQLite.
pub fn build_engine<S>(config: &ServerConfig, storage: S) -> eyre::Result<ArchitectAuth<S>> {
    let mut builder = ArchitectAuth::builder()
        .storage(storage)
        .secret(config.secret.clone())
        .base_url(config.base_url.clone())
        .oidc_issuer(config.issuer().to_owned())
        .session_ttl_seconds(config.session_ttl_seconds)
        .email_password_enabled(true)
        .require_email_verification(config.require_email_verification)
        .oidc_allow_dynamic_client_registration(config.oidc_allow_dynamic_client_registration)
        // PKCE is mandatory. Every first-party consumer here is either a
        // native app or an SPA — both are public clients that cannot keep
        // a secret, so the code-interception defence is the only one they
        // have.
        .oidc_require_pkce(true)
        .jwt_issuer(config.issuer().to_owned());

    if let Some(rp_id) = &config.passkey_rp_id {
        builder = builder
            .passkey_rp_id(rp_id.clone())
            .passkey_allowed_origin(config.base_url.clone());
    }

    for client in &config.oidc_clients {
        builder = builder.oidc_client(client.clone());
    }

    builder
        .build()
        .map_err(|error| eyre::eyre!("build ArchitectAuth: {error}"))
}

/// The full axum app: vox WebSocket + HTTP surface + health probes.
pub fn app_router<S>(config: &ServerConfig, auth: ArchitectAuth<S>) -> Router
where
    S: architect_auth::AuthStorage + Clone + Send + Sync + 'static,
{
    let cookie = cookie_config(config);
    // Mounted through the dispatcher rather than the plain
    // `auth_service_layer`, so `AuthServerMiddleware` parses the
    // `authorization` metadata entry off each call before the service
    // sees it — the same wrapping the token-store client middleware on
    // the app side expects.
    let vox_router = LayerRouter::new().with(
        architect_auth::auth_service_service_descriptor(),
        AuthServiceDispatcher::new(AuthVoxService::new(auth.clone()))
            .with_middleware(AuthServerMiddleware),
    );

    Router::new()
        .route(
            "/vox",
            get(move |ws: WebSocketUpgrade| {
                let router = vox_router.clone();
                async move {
                    ws.protocols([VOX_SUBPROTOCOL])
                        .on_upgrade(move |socket| architect::axum_ws::serve_router(socket, router))
                        .into_response()
                }
            }),
        )
        // Liveness: the process is up. Readiness is the same check for
        // now — the engine holds no lazily-initialised state, and a
        // database that has gone away surfaces as a 5xx on real traffic.
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .merge(http::router(HttpState::new(auth, cookie)))
        .layer(cors_layer(config))
        .layer(TraceLayer::new_for_http())
}

/// Cookie policy derived from the deployment.
///
/// `secure` follows the scheme: a `Secure` cookie is silently dropped
/// over plain HTTP, which would make local development mysteriously
/// fail to stay signed in.
fn cookie_config(config: &ServerConfig) -> AuthCookieConfig {
    AuthCookieConfig {
        secure: config.base_url.starts_with("https://"),
        max_age_seconds: Some(config.session_ttl_seconds),
        ..AuthCookieConfig::default()
    }
}

/// CORS for the browser front-ends.
///
/// With no configured origins the layer allows none — a same-origin
/// deployment needs no CORS, and defaulting to `Any` on an endpoint that
/// sets session cookies would be a real hole.
fn cors_layer(config: &ServerConfig) -> CorsLayer {
    if config.cors_origins.is_empty() {
        return CorsLayer::new();
    }
    let origins: Vec<_> = config
        .cors_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods(Any)
        .allow_headers(Any)
        // Required for the session cookie to ride cross-origin requests.
        .allow_credentials(true)
}

/// Bind and serve until the process dies.
pub async fn serve(server: AuthServer) -> eyre::Result<()> {
    let listener = tokio::net::TcpListener::bind(&server.bind_addr)
        .await
        .map_err(|error| eyre::eyre!("bind {}: {error}", server.bind_addr))?;
    tracing::info!(addr = %server.bind_addr, "auth server listening");
    axum::serve(listener, server.app)
        .await
        .map_err(|error| eyre::eyre!("serve: {error}"))
}
