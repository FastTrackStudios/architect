use chrono::{DateTime, Utc};
use uuid::Uuid;

#[auth_macros::entity(table_name = "auth_two_factors")]
#[derive(Eq)]
pub struct AuthTwoFactor {
    #[entity(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub user_id: Uuid,
    pub secret_ciphertext: String,
    pub backup_codes_hash: Option<String>,
    pub attempt_count: i64,
    #[entity(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[entity(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_two_factor_storage::{ActiveModel, Column, Entity, Model, Relation};
