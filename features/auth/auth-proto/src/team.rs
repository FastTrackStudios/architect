use chrono::{DateTime, Utc};
use uuid::Uuid;

#[auth_macros::entity(table_name = "auth_teams")]
#[derive(Eq)]
pub struct AuthTeam {
    #[entity(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    #[entity(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[entity(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_team_storage::{ActiveModel, Column, Entity, Model, Relation};
