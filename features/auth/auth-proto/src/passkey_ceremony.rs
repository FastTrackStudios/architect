use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The half-finished state of a `WebAuthn` ceremony.
///
/// A registration or an authentication is two round trips: the server
/// issues a challenge and must remember exactly what it issued, then
/// the browser answers and the server checks the answer against that
/// memory. `webauthn-rs` hands back an opaque state object for the gap.
///
/// # Why not the verification table
///
/// Every other short-lived secret here lives in `auth_verifications`,
/// whose `value_hash` is `.string()` — `varchar(255)` on `MySQL`. A
/// serialised ceremony state is several times that, so it would have
/// been silently truncated on one of the three supported backends and
/// worked on the other two. This is `.text()`.
///
/// # What is stored
///
/// The *hash* of the handle, like every other bearer value here. The
/// state itself is not a credential — completing a ceremony still
/// needs a valid signature — but a database dump should not let
/// somebody resume ceremonies either.
#[architect::entity(table_name = "auth_passkey_ceremonies", repo)]
#[derive(Eq)]
pub struct AuthPasskeyCeremony {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    /// `hash_token(secret, handle)` — never the handle.
    #[architect(filterable)]
    pub handle_hash: String,
    /// `registration` or `authentication`.
    #[architect(filterable)]
    pub kind: String,
    /// Who is registering. `None` for a discoverable sign-in, where the
    /// whole point is that the server does not yet know who is at the
    /// keyboard.
    #[architect(filterable)]
    pub user_id: Option<Uuid>,
    /// The serialised `webauthn-rs` state.
    pub state_json: String,
    #[architect(sortable)]
    pub expires_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

/// Which half of the passkey story a ceremony belongs to.
///
/// Stored as a string, and checked on completion: a registration state
/// must not be redeemable by the authentication endpoint or the other
/// way round, even though both are opaque blobs to this crate.
#[architect::wire]
#[derive(Copy, Eq)]
#[repr(u8)]
pub enum PasskeyCeremonyKind {
    Registration,
    Authentication,
}

impl PasskeyCeremonyKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Registration => "registration",
            Self::Authentication => "authentication",
        }
    }
}

#[cfg(feature = "server")]
pub use __auth_passkey_ceremony_storage::{ActiveModel, Column, Entity, Model, Relation};
