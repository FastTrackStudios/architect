//! Append-only record of every email an account has ever had.
//!
//! A user's email is not just a label: it is the login identifier, the
//! key operators recognise people by, and (across servers) the thing an
//! identity link is matched on. Overwriting it in place loses all of
//! that — after a migration there is no way to answer "who was
//! `old@example.com`?", which is exactly the question that comes up when
//! reconciling accounts, auditing access, or explaining why a link
//! stopped resolving.
//!
//! So every change appends a row here and the user row is updated. The
//! history is the migration trail: replaying it for a `user_id` gives the
//! full chain of addresses in order, and looking up a `previous_email`
//! answers the reverse question.
//!
//! Deliberately NOT a soft-delete or a versioned user table — the current
//! address stays on `auth_users` so every existing lookup, index and
//! uniqueness check keeps working untouched.

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg_attr(feature = "fake", derive(::fake::Dummy))]
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "auth_user_email_history", repo)]
pub struct AuthEmailChange {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    /// The account whose address changed. Stable across the whole chain —
    /// migrating an email never mints a new user, so anything keyed on
    /// this id (tasks, timers, authorship) stays attached.
    #[architect(filterable, sortable)]
    pub user_id: Uuid,
    /// The address held BEFORE this change. `None` only when the account
    /// had no email at all — the column on `auth_users` is nullable, so
    /// this one is too rather than inventing an empty string.
    #[architect(filterable)]
    pub previous_email: Option<String>,
    /// The address held AFTER this change.
    #[architect(filterable)]
    pub new_email: String,
    /// Who performed it. `None` = the user changed their own address;
    /// `Some(id)` = an operator or admin migrated it on their behalf.
    #[architect(filterable)]
    pub changed_by: Option<Uuid>,
    /// Free text, for migrations done in bulk ("consolidating onto the
    /// personal domain") so the trail explains itself later.
    pub reason: Option<String>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_email_change_storage::{ActiveModel, Column, Entity, Model, Relation};
