use chrono::{DateTime, Utc};
use uuid::Uuid;

#[architect::entity(table_name = "auth_organizations", repo)]
#[derive(Eq)]
pub struct AuthOrganization {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable, fulltext)]
    pub name: String,
    #[architect(filterable, sortable)]
    pub slug: String,
    pub logo: Option<String>,
    pub metadata_json: Option<String>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_organization_storage::{ActiveModel, Column, Entity, Model, Relation};
