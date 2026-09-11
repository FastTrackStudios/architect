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
    http::{HeaderValue, Method, header},
    response::IntoResponse,
    routing::get,
};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::config::ServerConfig;
use crate::http::{self, HttpState};
use crate::ui;

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

/// How to connect, given what the URL is.
///
/// # In-memory `SQLite` needs a pool of one
///
/// An in-memory `SQLite` database belongs to its *connection*, not to the
/// process. With the default pool the migrations run on one connection
/// and the second request is handed a different one — a database with
/// no tables in it — so the server boots, reports that it seeded, and
/// then fails every request with a 500.
///
/// It is the obvious URL to reach for on a dev machine and it looked
/// like it worked, because the first request often reuses the same
/// connection. Capping the pool at one makes `sqlite::memory:` mean
/// what everybody assumes it means.
fn connect_options(database_url: &str) -> sea_orm::ConnectOptions {
    let mut options = sea_orm::ConnectOptions::new(database_url.to_owned());
    if is_in_memory(database_url) {
        options.max_connections(1).min_connections(1);
    }
    options
}

/// Is this a `SQLite` database that lives only in this connection?
fn is_in_memory(database_url: &str) -> bool {
    let url = database_url.trim();
    url.starts_with("sqlite:")
        && (url.contains(":memory:") || url.contains("mode=memory"))
        // `cache=shared` makes one in-memory database visible to every
        // connection, which is the other way to solve this.
        && !url.contains("cache=shared")
}

/// Connect, migrate, and assemble everything from a [`ServerConfig`].
pub async fn build(config: &ServerConfig) -> eyre::Result<AuthServer> {
    let db = Database::connect(connect_options(&config.database_url))
        .await
        .map_err(|error| eyre::eyre!("connect auth database: {error}"))?;

    if config.run_migrations {
        tracing::info!("running auth migrations");
        Migrator::up(&db, None)
            .await
            .map_err(|error| eyre::eyre!("auth migrations: {error}"))?;
    }

    let auth = build_engine(config, AuthSeaOrmStorage::new(db.clone()))?;

    if let Some(path) = config.import_snapshot.as_deref() {
        match crate::dev::import_file(&db, &config.database_url, path).await {
            Ok(summary) => tracing::info!(
                target: "auth_server::dev",
                "imported {path}: {summary} — everyone's password is {:?}",
                crate::dev::DEV_PASSWORD,
            ),
            // Booting anyway would serve an empty server the operator
            // believes is a copy of production.
            Err(err) => return Err(eyre::eyre!("{err}")),
        }
    }

    if config.dev_seed {
        match crate::dev::seed(&auth, &config.database_url).await {
            Ok(true) => tracing::info!(
                target: "auth_server::dev",
                "seeded the development cast — sign in as {} with {:?}",
                crate::dev::DEV_PEOPLE[0].0,
                crate::dev::DEV_PASSWORD,
            ),
            Ok(false) => tracing::info!(
                target: "auth_server::dev",
                "database already has people in it; not seeding"
            ),
            // A refusal here is the local-database guard, and booting
            // anyway would serve a server the operator thought was
            // seeded. It is not a warning.
            Err(err) => return Err(eyre::eyre!("{err}")),
        }
    }

    let app = app_router_with_db(config, auth.clone(), db.clone())?;

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
/// in-memory `SQLite`.
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
        // Signing in with GitHub or Google for the first time creates the
        // account; without this the engine refuses unknown provider
        // identities and social sign-in only works for people who linked
        // first.
        .oauth_signup_enabled(true)
        // The scope a relying party must hold to be handed a linked
        // GitHub token. Registered here so a client that lists it can
        // actually be granted it at `/oauth2/authorize`.
        .oidc_extra_scope(config.social.linked_token_scope.clone())
        .jwt_issuer(config.issuer().to_owned());

    // One grantable scope per configured provider, for the same reason: a
    // client asking for `tone3000` can only be granted it if this server
    // knows the scope exists. Per provider rather than one blanket scope so
    // that being trusted to act as someone on TONE3000 does not also hand
    // over their GitHub token.
    for provider in crate::social::Provider::ALL {
        if config.social.provider_config_for(provider).is_some() {
            builder = builder.oidc_extra_scope(config.social.required_scope(provider).to_owned());
        }
    }

    // The engine checks the domain line of a signed message against
    // this, and the UI composes that line from the same value. Two
    // settings that must agree are two settings that will not, so both
    // come from `base_url`.
    builder = builder.siwe_domain(siwe_domain(config));

    if let Some(rp_id) = &config.passkey_rp_id {
        builder = builder
            .passkey_rp_id(rp_id.clone())
            // The origin a browser will report, which is this server's
            // own public URL. A mismatch here is not a subtle bug: the
            // browser refuses the ceremony outright.
            .passkey_allowed_origin(config.base_url.clone())
            // Shown by the operating system when it asks to create or
            // use a passkey. Without it the prompt says the domain,
            // which reads like a warning rather than an invitation.
            .passkey_rp_name("FastTrackStudio");
    }

    for client in &config.oidc_clients {
        builder = builder.oidc_client(client.clone());
    }

    builder
        .build()
        .map_err(|error| eyre::eyre!("build ArchitectAuth: {error}"))
}

/// The full axum app: vox WebSocket + HTTP surface + health probes.
///
/// # Errors
///
/// When social providers are configured but their HTTP client cannot be
/// built. That is a deployment which would send people to GitHub and
/// never finish, so it refuses to boot rather than serving a sign-in
/// button that dead-ends. With no provider configured the same failure
/// is not an error: nothing was going to use the client.
pub fn app_router<S>(config: &ServerConfig, auth: ArchitectAuth<S>) -> eyre::Result<Router>
where
    S: architect_auth::AuthStorage + Clone + Send + Sync + 'static,
{
    let social = match HttpState::<S>::social_state(config) {
        Ok(social) => social,
        Err(err) if config.social.is_enabled() => {
            return Err(eyre::eyre!(
                "social providers are configured but the HTTP client failed to build: {err}"
            ));
        }
        Err(_) => http::SocialState::disabled(),
    };
    Ok(app_router_with_social(
        config,
        auth,
        std::sync::Arc::new(social),
    ))
}

/// As [`app_router`], with the database attached so
/// `GET /admin/snapshot` can read it.
///
/// A separate function rather than a parameter on `app_router` because
/// the snapshot route is the only thing in the server that wants a
/// database handle rather than the storage trait, and every existing
/// caller — including every test — should keep getting a router
/// without it.
///
/// # Errors
///
/// As [`app_router`].
pub fn app_router_with_db<S>(
    config: &ServerConfig,
    auth: ArchitectAuth<S>,
    db: sea_orm::DatabaseConnection,
) -> eyre::Result<Router>
where
    S: architect_auth::AuthStorage + Clone + Send + Sync + 'static,
{
    let snapshot = Router::new()
        .route("/admin/snapshot", get(crate::dev::snapshot_route::<S>))
        .with_state(HttpState::new(auth.clone(), cookie_config(config)).with_db(db));
    Ok(app_router(config, auth)?.merge(snapshot))
}

/// As [`app_router`], with the social state supplied — the seam tests
/// use to swap the provider client for a fake.
pub fn app_router_with_social<S>(
    config: &ServerConfig,
    auth: ArchitectAuth<S>,
    social: std::sync::Arc<http::SocialState>,
) -> Router
where
    S: architect_auth::AuthStorage + Clone + Send + Sync + 'static,
{
    app_router_with_senders(config, auth, social, None, None)
}

/// As [`app_router_with_social`], with the senders supplied.
///
/// The seam a test needs: a sign-in code, a link and a phone code are
/// only observable through what was sent, and the router otherwise
/// builds its own senders from configuration — which in a test is log
/// mode, where nothing is observable at all. `None` keeps the built-in
/// behaviour for either one.
pub fn app_router_with_senders<S>(
    config: &ServerConfig,
    auth: ArchitectAuth<S>,
    social: std::sync::Arc<http::SocialState>,
    login_mailer: Option<std::sync::Arc<dyn auth_ui::mailer::LoginMailer>>,
    sms: Option<std::sync::Arc<dyn auth_ui::mailer::SmsSender>>,
) -> Router
where
    S: architect_auth::AuthStorage + Clone + Send + Sync + 'static,
{
    let cookie = cookie_config(config);

    // One mailer, shared by both routers. A failure to BUILD it (a bad
    // host, say) is a configuration error worth surfacing, but not worth
    // refusing to boot over: falling back to log mode keeps sign-in
    // working while mail is broken, which is the right way round.
    let mail = std::sync::Arc::new(match crate::mail::Mailer::new(config.mail.clone()) {
        Ok(mailer) => {
            if mailer.is_live() {
                tracing::info!(target: "auth_server::mail", from = %config.mail.from, "mail is live");
            } else {
                tracing::warn!(
                    target: "auth_server::mail",
                    "AUTH_SMTP_HOST is unset — verification and password-reset mail will be LOGGED, not sent"
                );
            }
            mailer
        }
        Err(err) => {
            tracing::error!(target: "auth_server::mail", %err, "mailer failed to build; falling back to log mode");
            crate::mail::Mailer::log_only(config.mail.clone())
        }
    });
    // The same mailer, seen through the trait `auth-ui`'s sign-in pages
    // use. One `Mailer`, two views of it — not two mailers.
    let mail_for_ui: std::sync::Arc<dyn auth_ui::mailer::LoginMailer> =
        login_mailer.unwrap_or_else(|| mail.clone());
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
        .merge(http::router(
            HttpState::new(auth.clone(), cookie.clone())
                .with_mailer(mail.clone())
                .with_social(social.clone()),
        ))
        // The sign-in and sign-up pages. Merged separately from the API
        // so an embedder that already has its own login screen can take
        // `http::router` alone — see `ui::router`.
        .merge(ui::router(
            HttpState::new(auth.clone(), cookie.clone())
                .with_mailer(mail)
                .with_social(social),
        ))
        // Account, organization and invitation pages. These live in
        // `auth-ui` rather than here because they are the same pages for
        // every deployment — a product wanting an org switcher should
        // mount them, not reimplement them.
        .merge(auth_ui::router(
            auth_ui::UiState::new(auth, cookie)
                .issuer("FastTrackStudio")
                .base_url(config.base_url.clone())
                .siwe_domain(siwe_domain(config))
                .mailer(mail_for_ui)
                .sms(sms.unwrap_or_else(auth_ui::mailer::log_only_sms)),
        ))
        .layer(cors_layer(config))
        .layer(TraceLayer::new_for_http())
}

/// Cookie policy derived from the deployment.
///
/// `secure` follows the scheme: a `Secure` cookie is silently dropped
/// over plain HTTP, which would make local development mysteriously
/// fail to stay signed in.
/// The host a wallet signature is bound to.
///
/// Derived from `base_url` rather than configured separately: it has to
/// be the host a browser reports, and two settings that must agree are
/// two settings that will not.
fn siwe_domain(config: &ServerConfig) -> String {
    config
        .base_url
        .split("://")
        .nth(1)
        .unwrap_or(&config.base_url)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[must_use]
pub fn cookie_config(config: &ServerConfig) -> AuthCookieConfig {
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
///
/// Methods and headers are enumerated rather than `Any`: the spec
/// forbids combining a wildcard with
/// `Access-Control-Allow-Credentials: true`, and tower-http enforces
/// that by panicking when the layer is built. Credentials are needed so
/// the session cookie can ride cross-origin requests from a browser
/// front-end on a different host.
fn cors_layer(config: &ServerConfig) -> CorsLayer {
    if config.cors_origins.is_empty() {
        return CorsLayer::new();
    }
    let origins: Vec<HeaderValue> = config
        .cors_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
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

#[cfg(test)]
mod tests {
    use super::is_in_memory;

    #[test]
    fn an_in_memory_sqlite_url_is_recognised() {
        // Each of these gives every connection its own empty database,
        // which is a server that boots and then 500s on every request.
        assert!(is_in_memory("sqlite::memory:"));
        assert!(is_in_memory("sqlite://:memory:"));
        assert!(is_in_memory("sqlite:file:x?mode=memory"));
    }

    #[test]
    fn a_shared_cache_url_solves_it_the_other_way() {
        assert!(!is_in_memory("sqlite:file:x?mode=memory&cache=shared"));
    }

    #[test]
    fn a_real_database_is_left_alone() {
        assert!(!is_in_memory("sqlite://./auth.db?mode=rwc"));
        assert!(!is_in_memory("postgres://auth@db/auth"));
    }
}
