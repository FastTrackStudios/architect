use chrono::{DateTime, Utc};
use uuid::Uuid;

#[architect::wire]
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

#[architect::entity(table_name = "auth_invitations", repo)]
#[derive(Eq)]
pub struct AuthInvitation {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub organization_id: Uuid,
    #[architect(filterable, sortable)]
    pub email: String,
    pub role: String,
    #[architect(filterable)]
    pub status: String,
    pub inviter_id: Uuid,
    pub expires_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_invitation_storage::{ActiveModel, Column, Entity, Model, Relation};
