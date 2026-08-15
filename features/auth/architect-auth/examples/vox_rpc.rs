use architect_auth::{
    ArchitectAuth, AuthServiceDispatcher, auth_service_layer,
    client::{MemoryTokenStore, StoredSession, TokenStore, TokenStoreMiddleware},
    db::{AuthSeaOrmStorage, Migrator},
    proto::architect::LayerRouter,
    transport::vox::{AuthClientMiddleware, AuthServerMiddleware, AuthVoxService},
};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let auth = ArchitectAuth::builder()
        .secret("a-secret-at-least-32-bytes-long!!")
        .storage(AuthSeaOrmStorage::new(db))
        .build()?;

    // Mount the session surface like any other architect service…
    let _router = LayerRouter::new().merge(auth_service_layer(AuthVoxService::new(auth.clone())));

    // …or build the dispatcher by hand to wrap it in middleware.
    let _dispatcher =
        AuthServiceDispatcher::new(AuthVoxService::new(auth)).with_middleware(AuthServerMiddleware);

    // Client side: attach a fixed token…
    let _client_middleware = AuthClientMiddleware::bearer("session-token");

    // …or use the session kit — persists the token (file-based on
    // native, in-memory here) and attaches it on every outgoing call.
    let store = std::sync::Arc::new(MemoryTokenStore::new());
    store.save(&StoredSession::new("session-token"))?;
    let _store_middleware = TokenStoreMiddleware::new(store);

    Ok(())
}
