use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg_attr(feature = "fake", derive(::fake::Dummy))]
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "auth_audit_events", repo)]
pub struct AuthAuditEventRecord {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub actor_id: Uuid,
    #[architect(filterable, sortable)]
    pub target_id: Option<Uuid>,
    #[architect(filterable, sortable)]
    pub action: String,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_audit_event_record_storage::{ActiveModel, Column, Entity, Model, Relation};
