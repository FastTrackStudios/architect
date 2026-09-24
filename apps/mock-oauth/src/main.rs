//! `mock-oauth` — run the stand-in.
//!
//! ```text
//! MOCK_OAUTH_BIND_ADDR=127.0.0.1:4040 mock-oauth
//! ```
//!
//! Then point an auth server at it with
//! `AUTH_SOCIAL_MOCK_URL=http://localhost:4040`.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,mock_oauth=debug".into()),
        )
        .init();

    let bind =
        std::env::var("MOCK_OAUTH_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:4040".to_owned());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::warn!(
        addr = %bind,
        "mock OAuth provider — nothing is verified here, never point anything real at it"
    );
    axum::serve(listener, mock_oauth::router()).await?;
    Ok(())
}
