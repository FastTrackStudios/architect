use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A shareable link that admits whoever opens it into an organization.
///
/// Distinct from [`crate::AuthInvitation`], which names one email
/// address, is accepted once, and moves through a status. A link names
/// nobody: it is handed out in a Slack channel or a course syllabus,
/// redeemed by everyone who follows it, and its interesting questions
/// are "how many more times" and "can I turn it off", which a status
/// enum cannot answer.
///
/// # What is stored
///
/// Only the *hash* of the token, under the same
/// `hash_token(secret, ..)` the session and invitation tokens use. A
/// stolen database dump therefore yields no working link, and the
/// consequence — that the plaintext is shown exactly once, at
/// creation — is the intended one.
#[architect::entity(table_name = "auth_invite_links", repo)]
#[derive(Eq)]
pub struct AuthInviteLink {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub organization_id: Uuid,
    /// `hash_token(secret, token)` — never the token.
    #[architect(filterable)]
    pub token_hash: String,
    /// An optional human label, so a list of links is readable.
    pub label: Option<String>,
    /// The role every member who joins through this link receives.
    pub role: String,
    pub created_by: Uuid,
    /// `None` means the link does not expire on its own.
    #[architect(sortable)]
    pub expires_at: Option<DateTime<Utc>>,
    /// `None` means unlimited.
    pub max_uses: Option<i32>,
    #[architect(exclude(create))]
    pub uses: i32,
    /// Set once, and never unset: a revoked link stays revoked so the
    /// same URL cannot be brought back to life after being shared
    /// somewhere it should not have been.
    pub revoked_at: Option<DateTime<Utc>>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}

impl AuthInviteLink {
    /// Whether this link will still admit somebody.
    ///
    /// Three independent reasons it might not, checked in the order a
    /// person would ask them: it was turned off, its time ran out, or
    /// it has been used up.
    #[must_use]
    pub fn is_usable_at(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|at| at > now)
            && self.max_uses.is_none_or(|max| self.uses < max)
    }

    /// How many uses remain, or `None` when the link is unlimited.
    #[must_use]
    pub fn uses_remaining(&self) -> Option<i32> {
        self.max_uses
            .map(|max| max.saturating_sub(self.uses).max(0))
    }
}

#[cfg(feature = "server")]
pub use __auth_invite_link_storage::{ActiveModel, Column, Entity, Model, Relation};

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use super::AuthInviteLink;

    fn link() -> AuthInviteLink {
        AuthInviteLink {
            id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            token_hash: "hash".into(),
            label: None,
            role: "member".into(),
            created_by: Uuid::new_v4(),
            expires_at: None,
            max_uses: None,
            uses: 0,
            revoked_at: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn a_plain_link_admits_forever() {
        let mut l = link();
        l.uses = 10_000;
        assert!(l.is_usable_at(Utc::now()));
        assert_eq!(l.uses_remaining(), None);
    }

    #[test]
    fn a_revoked_link_stays_shut_even_with_uses_left() {
        let mut l = link();
        l.max_uses = Some(10);
        l.revoked_at = Some(Utc::now());
        assert!(!l.is_usable_at(Utc::now()));
    }

    #[test]
    fn a_spent_link_reports_no_uses_remaining() {
        let mut l = link();
        l.max_uses = Some(2);
        l.uses = 2;
        assert!(!l.is_usable_at(Utc::now()));
        assert_eq!(l.uses_remaining(), Some(0));
    }

    #[test]
    fn an_over_spent_link_does_not_report_a_negative_remainder() {
        let mut l = link();
        l.max_uses = Some(2);
        l.uses = 5;
        assert_eq!(l.uses_remaining(), Some(0));
    }

    #[test]
    fn an_expired_link_is_shut_and_a_future_one_is_open() {
        let mut l = link();
        l.expires_at = Some(Utc::now() - Duration::hours(1));
        assert!(!l.is_usable_at(Utc::now()));
        l.expires_at = Some(Utc::now() + Duration::hours(1));
        assert!(l.is_usable_at(Utc::now()));
    }
}
