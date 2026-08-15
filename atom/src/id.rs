//! [`Id`] — a row's identity in the store: a real server key, or a
//! client-forged placeholder for an in-flight optimistic insert.
//!
//! A typed enum — *not* a magic string prefix like `"tmp-{n}"`. A prefix
//! is brittle (a real id that happens to start with `tmp-` is
//! misclassified) and the dioxus-MCP `magic_id_prefix_for_optimistic`
//! audit flags it; the enum is the audit's own recommended fix and stays
//! `Copy` when the key is (e.g. `Uuid`).

/// Identity of a row held in a [`Store`](crate::Store).
///
/// `Temp` ids come from the store's monotonic sequence, so two rapid
/// optimistic inserts never collide. On server confirmation a `Temp` id is
/// swapped for the authoritative `Real` id (see
/// [`Store::reconcile`](crate::Store::reconcile)).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Id<K> {
    /// Server-assigned, canonical identity.
    Real(K),
    /// Client-forged placeholder for an optimistic insert not yet
    /// confirmed by the server.
    Temp(u64),
}

impl<K: std::fmt::Display> std::fmt::Display for Id<K> {
    /// A stable string for use as a Dioxus list `key` (not an entity id):
    /// the real key, or `temp-{n}` for an optimistic placeholder.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Real(k) => write!(f, "{k}"),
            Self::Temp(n) => write!(f, "temp-{n}"),
        }
    }
}

impl<K> Id<K> {
    /// The real key, or `None` for a still-optimistic temp row.
    pub fn real(&self) -> Option<&K> {
        match self {
            Self::Real(k) => Some(k),
            Self::Temp(_) => None,
        }
    }

    /// True for a client-forged placeholder id.
    pub fn is_temp(&self) -> bool {
        matches!(self, Self::Temp(_))
    }

    /// True for a server-assigned id.
    pub fn is_real(&self) -> bool {
        matches!(self, Self::Real(_))
    }
}
