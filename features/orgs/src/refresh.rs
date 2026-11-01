//! When the person's orgs are fetched again: on every (re)connect.
//!
//! The list is kept on the device (the cache [`crate::Orgs`] round-trips);
//! a connection's generation (`architect_atom::Connection::generation`)
//! counts its establishes, so the list is fetched once per generation and
//! a failed fetch keeps the cached list.

use crate::model::Org;

/// Which connection the list was last fetched on, and the last failure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refresh {
    fetched: Option<u64>,
    error: Option<String>,
}

impl Refresh {
    /// Whether the list is due on connection `generation`: not yet
    /// fetched on it (a reconnect, or the first connect).
    #[must_use]
    pub fn due(&self, generation: u64) -> bool {
        self.fetched.is_none_or(|g| g < generation)
    }

    /// The fetch on `generation` answered: the fresh list to keep, or
    /// `None` when it failed (the cached list stays, and the next
    /// (re)connect asks again).
    pub fn finish(
        &mut self,
        generation: u64,
        answer: Result<Vec<Org>, String>,
    ) -> Option<Vec<Org>> {
        match answer {
            Ok(orgs) => {
                self.fetched = Some(generation);
                self.error = None;
                Some(orgs)
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// Why the list shown is the cached one, if the last fetch failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The connection the list was last fetched on.
    #[must_use]
    pub const fn fetched(&self) -> Option<u64> {
        self.fetched
    }
}
