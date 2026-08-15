use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg_attr(feature = "fake", derive(::fake::Dummy))]
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "auth_api_keys", repo)]
pub struct AuthApiKey {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub name: Option<String>,
    pub prefix: Option<String>,
    pub key_hash: String,
    #[architect(filterable, sortable)]
    pub user_id: Uuid,
    pub enabled: bool,
    pub rate_limit_enabled: bool,
    pub rate_limit_time_window: Option<i64>,
    pub rate_limit_max: Option<i64>,
    pub request_count: Option<i64>,
    pub remaining: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub permissions_json: Option<String>,
    pub metadata_json: Option<String>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_api_key_storage::{ActiveModel, Column, Entity, Model, Relation};
