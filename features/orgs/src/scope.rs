//! What a read or a write is in: the scope files, documents, settings and
//! presence key their data by, and the All-organizations read across it.
//!
//! One convention for every layer: an org's data lives under
//! `org/<id>/…` ([`OrgId::path`]) — a blob root, a settings file, a
//! permission resource — and an org's own documents (its roster, its
//! catalogue) have ids derived from the org's ([`OrgId::doc_id`]). A
//! window's [`Scope`] says which orgs a read covers; a write always goes
//! to one ([`Scope::writes_to`]).

use std::future::Future;

use uuid::Uuid;

use crate::model::{OrgId, Orgs, Selection};

/// The prefix every org-scoped path starts with.
const PREFIX: &str = "org/";

/// The namespace an org's document ids are derived in (a v5 UUID of
/// `architect.orgs`), so two apps derive the same id for the same org.
const DOC_NAMESPACE: Uuid = Uuid::from_bytes([
    0x5b, 0x0e, 0x6c, 0x2a, 0x81, 0x3f, 0x5d, 0x4e, 0x9a, 0x77, 0x14, 0x0c, 0xe2, 0x63, 0x48, 0xd1,
]);

impl OrgId {
    /// `rest` within this org: `org/<id>/<rest>` (`org/<id>` for an empty
    /// `rest`). The key a layer stores the org's data under, and the
    /// resource a permission names it by.
    #[must_use]
    pub fn path(&self, rest: &str) -> String {
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() {
            format!("{PREFIX}{}", self.0)
        } else {
            format!("{PREFIX}{}/{rest}", self.0)
        }
    }

    /// The org a path is in, and the rest of it: the inverse of
    /// [`Self::path`]. `None` for a path in no org.
    #[must_use]
    pub fn of_path(path: &str) -> Option<(Self, &str)> {
        let tail = path.strip_prefix(PREFIX)?;
        let (id, rest) = tail.split_once('/').unwrap_or((tail, ""));
        (!id.is_empty()).then(|| (Self::new(id), rest))
    }

    /// The id of this org's own document `name` (`presence`, the org's
    /// roster; `catalogue`): the same in every app and on every device,
    /// and different in every org.
    #[must_use]
    pub fn doc_id(&self, name: &str) -> Uuid {
        Uuid::new_v5(&DOC_NAMESPACE, self.path(name).as_bytes())
    }
}

/// The orgs a read covers: the window's one, or every org the person is
/// in (All). Writing always picks one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    One(OrgId),
    /// Every org, in the person's order.
    All(Vec<OrgId>),
}

impl Scope {
    /// The orgs a read asks.
    #[must_use]
    pub fn orgs(&self) -> &[OrgId] {
        match self {
            Self::One(id) => std::slice::from_ref(id),
            Self::All(ids) => ids,
        }
    }

    #[must_use]
    pub fn contains(&self, org: &OrgId) -> bool {
        self.orgs().contains(org)
    }

    /// Where a write goes: the one org, or none in All (the person picks
    /// one first).
    #[must_use]
    pub const fn writes_to(&self) -> Option<&OrgId> {
        match self {
            Self::One(id) => Some(id),
            Self::All(_) => None,
        }
    }

    #[must_use]
    pub const fn is_all(&self) -> bool {
        matches!(self, Self::All(_))
    }
}

impl Orgs {
    /// What the window's reads cover: its org, or in All every org.
    #[must_use]
    pub fn scope(&self) -> Scope {
        match &self.selection {
            Selection::One(id) if self.get(id).is_some() => Scope::One(id.clone()),
            _ => Scope::All(self.orgs.iter().map(|o| o.id.clone()).collect()),
        }
    }
}

/// One item and the org it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scoped<T> {
    pub org: OrgId,
    pub item: T,
}

/// What a read across orgs found: every org's items, in the scope's
/// order, and the orgs that could not answer (offline, refused). One org
/// failing hides none of the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate<T, E> {
    pub items: Vec<Scoped<T>>,
    pub failed: Vec<(OrgId, E)>,
}

impl<T, E> Default for Aggregate<T, E> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            failed: Vec::new(),
        }
    }
}

impl<T, E> Aggregate<T, E> {
    /// Every org's answer, in the order asked.
    pub fn collect(answers: impl IntoIterator<Item = (OrgId, Result<Vec<T>, E>)>) -> Self {
        let mut all = Self::default();
        for (org, answer) in answers {
            match answer {
                Ok(items) => all.items.extend(items.into_iter().map(|item| Scoped {
                    org: org.clone(),
                    item,
                })),
                Err(e) => all.failed.push((org, e)),
            }
        }
        all
    }

    /// Whether every org answered.
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Reads `read` from every org in `scope` at once (the library across
/// orgs, ⌘O across them); in a one-org scope, from that org.
pub async fn read_all<T, E, F, Fut>(scope: &Scope, read: F) -> Aggregate<T, E>
where
    F: Fn(OrgId) -> Fut,
    Fut: Future<Output = Result<Vec<T>, E>>,
{
    let orgs = scope.orgs().to_vec();
    let answers = futures_util::future::join_all(orgs.iter().cloned().map(read)).await;
    Aggregate::collect(orgs.into_iter().zip(answers))
}
