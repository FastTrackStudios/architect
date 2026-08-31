//! `auth-server` — run the FastTrackStudio identity server.
//!
//! Configuration is entirely environmental, so the same image serves
//! every environment. See [`auth_server::config`] for the full list;
//! the two required vars are `AUTH_SECRET` (or `AUTH_SECRET_FILE`) and
//! `AUTH_DATABASE_URL` (or `AUTH_DATABASE_URL_FILE`).

use auth_server::{ServerConfig, server};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    architect::host::init_tracing("info,auth_server=debug");
    architect::host::install_panic_logger();

    let config = ServerConfig::from_env()?;
    tracing::info!(
        base_url = %config.base_url,
        issuer = %config.issuer(),
        oidc_clients = config.oidc_clients.len(),
        "starting auth server"
    );

    let built = server::build(&config).await?;
    server::serve(built).await
}
