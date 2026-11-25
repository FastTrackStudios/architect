use chrono::{DateTime, Utc};
use uuid::Uuid;

#[auth_macros::entity(table_name = "auth_audit_events")]
#[derive(Eq)]
pub struct AuthAuditEventRecord {
    #[entity(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub actor_id: Uuid,
    pub target_id: Option<Uuid>,
    pub action: String,
    #[entity(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_audit_event_record_storage::{ActiveModel, Column, Entity, Model, Relation};
