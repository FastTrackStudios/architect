//! Assembling the running server: storage, engine, the services' two
//! faces, the plugins, the listener.
//!
//! The auth *services* are declared once as `#[auth_macros::service]`
//! traits in `auth-proto`; from that declaration come the vox
//! dispatchers, the axum routes and the clients. This module mounts them:
//!
//! ```text
//! vox_router(auth)   →  the vox router     (served at /vox)
//! http_router(auth)  →  the HTTP+JSON face (POST /auth/…)
//! Host::new(vox).plugin(http).plugin(oauth).plugin(pages)…
//! ```
//!
//! What remains hand-written is what is HTTP by nature: the OAuth/OIDC
//! redirect flows ([`crate::oauth`]) and the server-rendered pages
//! ([`crate::ui`], `auth_ui`). Those mount as plugins next to the
//! services.

use architect_auth::vox_host::VoxRouter;
use architect_auth::{
    ArchitectAuth, AuthStorage,
    db::{AuthSeaOrmStorage, Migrator},
    transport::{AuthCookieConfig, vox::AuthVoxService},
};
use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::get,
};
use sea_orm::DatabaseConnection;
use sea_orm_migration::MigratorTrait as _;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::config::ServerConfig;
use crate::host::Host;
use crate::oauth::{self, HttpState};
use crate::ui;

/// The vox WebSocket subprotocol `/vox` echoes.
pub use crate::host::VOX_SUBPROTOCOL;

/// A built, not-yet-listening server.
pub struct AuthServer {
    pub auth: ArchitectAuth<AuthSeaOrmStorage>,
    /// Everything mounted, ready to bind. [`Host::into_app`] hands back
    /// the axum app for in-process use; [`serve`] binds it.
    pub host: Host,
    /// Kept so callers (tests, an embedding binary) can reach the same
    /// pool the engine writes through.
    pub db: DatabaseConnection,
}

/// The vox router: every service, answered by the one backend that
/// implements them all. What `/vox` and an in-process connection serve.
#[must_use]
pub fn vox_router<S>(auth: ArchitectAuth<S>) -> VoxRouter
where
    S: AuthStorage,
{
    let svc = AuthVoxService::new(auth);
    VoxRouter::new()
        .with(
            auth_proto::auth_service_service_descriptor(),
            auth_proto::AuthServiceDispatcher::new(svc.clone()),
        )
        .with(
            auth_proto::organization_service_service_descriptor(),
            auth_proto::OrganizationServiceDispatcher::new(svc.clone()),
        )
        .with(
            auth_proto::device_service_service_descriptor(),
            auth_proto::DeviceServiceDispatcher::new(svc),
        )
}

/// The HTTP+JSON face: `POST /auth/<method>`, JSON in, JSON out, the
/// session token in the body or as `Authorization: Bearer`.
#[must_use]
pub fn http_router<S>(auth: ArchitectAuth<S>) -> Router
where
    S: AuthStorage,
{
    let svc = std::sync::Arc::new(AuthVoxService::new(auth));
    let router = auth_proto::auth_service_http::mount(svc.clone(), Router::new());
    let router = auth_proto::organization_service_http::mount(svc.clone(), router);
    auth_proto::device_service_http::mount(svc, router)
}

/// The database at `url`. An in-memory `SQLite` database lives in one
/// connection, so its pool holds exactly one (a second would be empty).
async fn connect(url: &str) -> Result<DatabaseConnection, sea_orm::DbErr> {
    let mut options = sea_orm::ConnectOptions::new(url.to_owned());
    let url = url.trim();
    let in_memory = url.starts_with("sqlite:")
        && (url.contains(":memory:") || url.contains("mode=memory"))
        && !url.contains("cache=shared");
    if in_memory {
        options.max_connections(1).min_connections(1);
    }
    sea_orm::Database::connect(options).await
}

/// Connect, migrate, and assemble everything from a [`ServerConfig`].
pub async fn build(config: &ServerConfig) -> eyre::Result<AuthServer> {
    // The dev stack's auto sign-in, refused before anything opens: a
    // server asked for it anywhere but the stack's own state on loopback
    // does not start (`crate::dev::auto_sign_in`).
    match crate::dev::auto_sign_in(config) {
        Ok(Some(_)) => tracing::info!(
            target: "auth_server::dev",
            "the dev stack's demo account is signed in by itself at /oauth2/authorize"
        ),
        Ok(None) => {}
        Err(why) => {
            tracing::warn!(target: "auth_server::dev", %why, "AUTH_DEV_AUTO_SIGN_IN refused");
            return Err(eyre::eyre!("AUTH_DEV_AUTO_SIGN_IN refused: {why}"));
        }
    }
    let db = connect(&config.database_url)
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

    let social = social_state(config)?;
    let host = host_with_senders(
        config,
        auth.clone(),
        std::sync::Arc::new(social),
        None,
        None,
    )
    .plugin(snapshot_plugin(config, auth.clone(), db.clone()));

    Ok(AuthServer { auth, host, db })
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

/// The social-provider state, or why it could not be built.
///
/// # Errors
///
/// When social providers are configured but their HTTP client cannot be
/// built. That is a deployment which would send people to GitHub and
/// never finish, so it refuses to boot rather than serving a sign-in
/// button that dead-ends. With no provider configured the same failure
/// is not an error: nothing was going to use the client.
fn social_state(config: &ServerConfig) -> eyre::Result<oauth::SocialState> {
    match HttpState::<AuthSeaOrmStorage>::social_state(config) {
        Ok(social) => Ok(social),
        Err(err) if config.social.is_enabled() => Err(eyre::eyre!(
            "social providers are configured but the HTTP client failed to build: {err}"
        )),
        Err(_) => Ok(oauth::SocialState::disabled()),
    }
}

/// The full axum app: vox WebSocket + generated HTTP face + plugins +
/// health probes. What every in-process test drives.
///
/// # Errors
///
/// As [`social_state`].
pub fn app_router<S>(config: &ServerConfig, auth: ArchitectAuth<S>) -> eyre::Result<Router>
where
    S: AuthStorage,
{
    let social = social_state(config)?;
    Ok(app_router_with_social(
        config,
        auth,
        std::sync::Arc::new(social),
    ))
}

/// As [`app_router`], with the database attached so
/// `GET /admin/snapshot` can read it.
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
    S: AuthStorage,
{
    let social = social_state(config)?;
    Ok(host_with_senders(
        config,
        auth.clone(),
        std::sync::Arc::new(social),
        None,
        None,
    )
    .plugin(snapshot_plugin(config, auth, db))
    .into_app())
}

/// As [`app_router`], with the social state supplied — the seam tests
/// use to swap the provider client for a fake.
pub fn app_router_with_social<S>(
    config: &ServerConfig,
    auth: ArchitectAuth<S>,
    social: std::sync::Arc<oauth::SocialState>,
) -> Router
where
    S: AuthStorage,
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
    social: std::sync::Arc<oauth::SocialState>,
    login_mailer: Option<std::sync::Arc<dyn auth_ui::mailer::LoginMailer>>,
    sms: Option<std::sync::Arc<dyn auth_ui::mailer::SmsSender>>,
) -> Router
where
    S: AuthStorage,
{
    host_with_senders(config, auth, social, login_mailer, sms).into_app()
}

/// `GET /admin/snapshot` — the one route that wants the database rather
/// than the storage trait. A plugin of its own so every other caller
/// gets a host without it.
fn snapshot_plugin<S>(
    config: &ServerConfig,
    auth: ArchitectAuth<S>,
    db: sea_orm::DatabaseConnection,
) -> Router
where
    S: AuthStorage,
{
    Router::new()
        .route("/admin/snapshot", get(crate::dev::snapshot_route::<S>))
        .with_state(HttpState::new(auth, cookie_config(config)).with_db(db))
}

/// Liveness and readiness. Readiness is the same check for now — the
/// engine holds no lazily-initialised state, and a database that has
/// gone away surfaces as a 5xx on real traffic. `/health` comes from
/// the host; these are the names the deployment probes.
fn probes() -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
}

/// The host, assembled: the generated faces plus every plugin.
fn host_with_senders<S>(
    config: &ServerConfig,
    auth: ArchitectAuth<S>,
    social: std::sync::Arc<oauth::SocialState>,
    login_mailer: Option<std::sync::Arc<dyn auth_ui::mailer::LoginMailer>>,
    sms: Option<std::sync::Arc<dyn auth_ui::mailer::SmsSender>>,
) -> Host
where
    S: AuthStorage,
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

    let cors = cors_layer(config);
    // `build` refused a server that asks for it and may not; an embedder
    // that skipped `build` gets the form instead.
    let auto_sign_in = crate::dev::auto_sign_in(config).unwrap_or_else(|why| {
        tracing::warn!(target: "auth_server::dev", %why, "AUTH_DEV_AUTO_SIGN_IN refused: the sign-in form instead");
        None
    });

    // Ungated on purpose: this server IS the identity provider. Signing
    // up and signing in are anonymous by nature, and every other method
    // takes the session token as an argument and checks it itself, on
    // every face (the HTTP face has no gate either). A gate here would
    // have no identity to resolve that the services don't already check.
    Host::new(vox_router(auth.clone()), config.bind_addr.clone())
        // The same services, as HTTP+JSON.
        .plugin(http_router(auth.clone()))
        .plugin(probes())
        // The OAuth flows: this server as an OIDC provider, and as a
        // client of GitHub / Google / TONE3000.
        .plugin(oauth::router(
            HttpState::new(auth.clone(), cookie.clone())
                .with_mailer(mail.clone())
                .with_social(social.clone())
                .with_dev_auto_sign_in(auto_sign_in),
        ))
        // The sign-in and sign-up pages. A plugin of their own so an
        // embedder that already has a login screen can leave them out.
        .plugin(ui::router(
            HttpState::new(auth.clone(), cookie.clone())
                .with_mailer(mail)
                .with_social(social),
        ))
        // Account, organization and invitation pages. These live in
        // `auth-ui` rather than here because they are the same pages for
        // every deployment — a product wanting an org switcher should
        // mount them, not reimplement them.
        .plugin(auth_ui::router(
            auth_ui::UiState::new(auth, cookie)
                .issuer("FastTrackStudio")
                .base_url(config.base_url.clone())
                .siwe_domain(siwe_domain(config))
                .mailer(mail_for_ui)
                .sms(sms.unwrap_or_else(auth_ui::mailer::log_only_sms)),
        ))
        .finish(move |app| app.layer(cors).layer(TraceLayer::new_for_http()))
}

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

/// Cookie policy derived from the deployment.
///
/// `secure` follows the scheme: a `Secure` cookie is silently dropped
/// over plain HTTP, which would make local development mysteriously
/// fail to stay signed in.
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

/// Bind and serve until the process dies: `/vox`, the HTTP face, every
/// plugin.
pub async fn serve(server: AuthServer) -> eyre::Result<()> {
    server
        .host
        .serve()
        .await
        .map_err(|error| eyre::eyre!("serve: {error}"))
}
