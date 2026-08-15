use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg_attr(feature = "fake", derive(::fake::Dummy))]
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "auth_members", repo)]
pub struct AuthMember {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub organization_id: Uuid,
    #[architect(filterable, sortable)]
    pub user_id: Uuid,
    #[architect(filterable)]
    pub role: String,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_member_storage::{ActiveModel, Column, Entity, Model, Relation};
