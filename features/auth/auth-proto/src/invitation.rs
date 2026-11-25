use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Eq)]
#[repr(u8)]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Rejected,
    Canceled,
}

impl InvitationStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Canceled => "canceled",
        }
    }
}

#[auth_macros::entity(table_name = "auth_invitations")]
#[derive(Eq)]
pub struct AuthInvitation {
    #[entity(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub organization_id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub inviter_id: Uuid,
    pub expires_at: DateTime<Utc>,
    #[entity(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_invitation_storage::{ActiveModel, Column, Entity, Model, Relation};
