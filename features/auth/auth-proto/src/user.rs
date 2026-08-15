use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg_attr(feature = "fake", derive(::fake::Dummy))]
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "auth_users", repo)]
pub struct AuthUser {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub email: Option<String>,
    #[architect(filterable, sortable, fulltext)]
    pub name: Option<String>,
    pub email_verified: bool,
    pub image: Option<String>,
    #[architect(filterable, sortable)]
    pub username: Option<String>,
    pub display_username: Option<String>,
    pub two_factor_enabled: bool,
    #[architect(filterable)]
    pub role: Option<String>,
    pub banned: bool,
    pub ban_reason: Option<String>,
    pub ban_expires: Option<DateTime<Utc>>,
    pub metadata_json: String,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_user_storage::{ActiveModel, Column, Entity, Model, Relation};
