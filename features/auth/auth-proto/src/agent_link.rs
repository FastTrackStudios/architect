use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A standing grant from one account to another: "this agent may act
/// wherever I may, up to this role".
///
/// The reason it exists is the AI assistant a person runs. It has its
/// own account, so everything it writes is attributed to it and not to
/// the person; but every organization the person joins would otherwise
/// need a second invitation for the agent, and the person forgets, and
/// the agent is locked out of exactly the room it was asked to work in.
/// A link answers that once: the agent inherits the owner's memberships
/// as they change, with the role lowered to `max_role` where the owner's
/// is higher.
///
/// `max_role` is `admin` or `member`. Owner is never inheritable: an
/// agent cannot delete or hand over an organization on somebody's
/// behalf, whatever it was told.
#[architect::entity(table_name = "auth_agent_links", repo)]
#[derive(Eq)]
pub struct AuthAgentLink {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    /// The person whose memberships are inherited.
    #[architect(filterable, sortable)]
    pub owner_user_id: Uuid,
    /// The account doing the inheriting.
    #[architect(filterable, sortable)]
    pub agent_user_id: Uuid,
    /// The highest role the agent may hold through this link.
    pub max_role: String,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_agent_link_storage::{ActiveModel, Column, Entity, Model, Relation};
