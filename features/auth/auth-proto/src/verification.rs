use chrono::{DateTime, Utc};
use uuid::Uuid;

#[auth_macros::entity(table_name = "auth_verifications")]
#[derive(Eq)]
pub struct AuthVerification {
    #[entity(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub identifier: String,
    pub value_hash: String,
    pub expires_at: DateTime<Utc>,
    #[entity(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[entity(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_verification_storage::{ActiveModel, Column, Entity, Model, Relation};
