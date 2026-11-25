use chrono::{DateTime, Utc};
use uuid::Uuid;

#[auth_macros::entity(table_name = "auth_sessions")]
#[derive(Eq)]
pub struct AuthSession {
    #[entity(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub impersonated_by: Option<Uuid>,
    pub active_organization_id: Option<Uuid>,
    pub active: bool,
    #[entity(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[entity(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_session_storage::{ActiveModel, Column, Entity, Model, Relation};
