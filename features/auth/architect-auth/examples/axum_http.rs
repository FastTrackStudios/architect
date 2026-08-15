use architect_auth::{
    ArchitectAuth,
    db::{AuthSeaOrmStorage, Migrator},
    transport::axum::{AuthenticatedSession, AxumAuthState, require_session},
};
use axum::{Extension, Router, middleware, routing::get};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

async fn current_user(Extension(session): Extension<AuthenticatedSession>) -> String {
    session.user_id.to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let auth = ArchitectAuth::builder()
        .secret("a-secret-at-least-32-bytes-long!!")
        .storage(AuthSeaOrmStorage::new(db))
        .build()?;

    let _app: Router = Router::new()
        .route("/api/me", get(current_user))
        .route_layer(middleware::from_fn_with_state(
            AxumAuthState::new(auth),
            require_session,
        ));

    Ok(())
}
