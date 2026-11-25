use chrono::{DateTime, Utc};
use uuid::Uuid;

#[auth_macros::entity(table_name = "auth_team_members")]
#[derive(Eq)]
pub struct AuthTeamMember {
    #[entity(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub team_id: Uuid,
    pub user_id: Uuid,
    #[entity(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_team_member_storage::{ActiveModel, Column, Entity, Model, Relation};
