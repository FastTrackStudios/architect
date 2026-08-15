use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg_attr(feature = "fake", derive(::fake::Dummy))]
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "auth_accounts", repo)]
pub struct AuthAccount {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable)]
    pub account_id: String,
    #[architect(filterable, sortable)]
    pub provider_id: String,
    #[architect(filterable, sortable)]
    pub user_id: Uuid,
    pub access_token_ciphertext: Option<String>,
    pub refresh_token_ciphertext: Option<String>,
    pub id_token_ciphertext: Option<String>,
    pub access_token_expires_at: Option<DateTime<Utc>>,
    pub refresh_token_expires_at: Option<DateTime<Utc>>,
    pub scope: Option<String>,
    pub password_hash: Option<String>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_account_storage::{ActiveModel, Column, Entity, Model, Relation};
