use chrono::{DateTime, Utc};
use uuid::Uuid;

#[architect::entity(table_name = "auth_passkeys", repo)]
#[derive(Eq)]
pub struct AuthPasskey {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub name: String,
    #[architect(filterable, sortable)]
    pub user_id: Uuid,
    pub public_key: String,
    #[architect(filterable)]
    pub credential_id: String,
    pub counter: i64,
    pub device_type: String,
    pub backed_up: bool,
    pub transports: Option<String>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_passkey_storage::{ActiveModel, Column, Entity, Model, Relation};
