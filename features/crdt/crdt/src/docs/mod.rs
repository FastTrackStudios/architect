//! `crdt::docs` — a catalog of schema-agnostic documents, and the lanes
//! that keep it on every device.
//!
//! An app keeps its data as Loro docs (a project, a setlist, a rig) and
//! wants each of a person's devices to hold them, offline-first, with
//! the host never knowing what is inside one. The pieces:
//!
//! - **[`DocMeta`]**: what a doc is, without opening it. Its id is
//!   chosen by the device that made it (so a doc can be made offline and
//!   registered later), its `kind` is an opaque string the app picks
//!   (`session`, `setlist`, …), and `parent` names the doc it was forked
//!   from (carried, never interpreted).
//! - **[`Docs`]**: the wire. `create` (idempotent on the id), `get`,
//!   `list` (since a catalog revision), `rename`, `delete` (a tombstone,
//!   so an offline device learns of it) and the `changes` stream.
//!   Content does not ride this lane: a doc's bytes sync over
//!   [`DocSync`](crate::sync::DocSync), addressed by the same id.
//! - **[`DocsStore`]**: where a host keeps the catalog. One trait, a
//!   backend per deployment, like [`Persistence`](crate::Persistence)
//!   for the bytes: [`InMemoryDocsStore`] here, `SeaOrmDocsStore` in
//!   `crdt-seaorm`.
//! - **`DocsHost`** (native): the catalog served (`Docs` + its stream)
//!   with the content lane beside it (`DocSync` + `DocPresence` over a
//!   [`DocRegistry`](crate::DocRegistry)), admitting only catalogued,
//!   live ids.
//! - **`DocsDriver`** (native): the device side. It keeps the device's
//!   own index ([`DocsIndex`]) and a persistence per doc
//!   ([`DocStorage`]), creates offline, and on each connection registers
//!   pending creates (parent first), catches the catalog up, and syncs
//!   every doc durably until the connection drops.
//!
//! ## Revisions
//!
//! Every catalog write takes the next revision of its store (`rev`,
//! contiguous from 1). The client contract is: subscribe to `changes`,
//! wait for its first frame ([`DocChange::Attached`], the head at
//! attach), then `list { since_rev: last_seen }` and fold both, dropping
//! anything at or below what it has. A frame whose `rev` skips one means
//! the stream dropped events: list again from the last one seen.

use uuid::Uuid;

use crate::persistence::MaybeSendSync;

#[cfg(not(target_arch = "wasm32"))]
mod device_host;
#[cfg(not(target_arch = "wasm32"))]
mod driver;
#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(all(not(target_arch = "wasm32"), feature = "turso"))]
mod turso;

#[cfg(all(not(target_arch = "wasm32"), feature = "turso"))]
pub use self::turso::TursoDocsIndex;
#[cfg(not(target_arch = "wasm32"))]
pub use device_host::DriverHost;
#[cfg(not(target_arch = "wasm32"))]
pub use driver::{
    DocStorage, DocSyncEvent, DocsDriver, DocsIndex, DocsLane, HUB, InMemoryDocsIndex, LocalDoc,
    Refusal, SharedStorage,
};
#[cfg(not(target_arch = "wasm32"))]
pub use host::{DocAccess, DocsHost};

/// Longest `kind` a catalog accepts.
pub const MAX_KIND_LEN: usize = 64;
/// Longest `name` a catalog accepts, in characters.
pub const MAX_NAME_LEN: usize = 512;

/// What a document is, without opening it.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct DocMeta {
    /// Chosen by the device that made it.
    pub doc_id: Uuid,
    /// The app's word for what this is. Opaque to the host; checked
    /// only for shape (1 to [`MAX_KIND_LEN`] of `[a-z0-9._-]`).
    pub kind: String,
    /// What a list shows. Opaque.
    pub name: String,
    /// The doc this one was forked from. Carried, never interpreted.
    pub parent: Option<Uuid>,
    /// Who registered it: the caller's user id (or its principal's
    /// description when it is not a user), stamped by the host. Empty
    /// on a device's own not-yet-registered row.
    pub created_by: String,
    /// The peer node that registered it, when a node (not a person's
    /// client) did.
    pub origin: Option<String>,
    /// Microseconds since the Unix epoch, host clock.
    pub created_us: i64,
    /// Last catalog change (rename, delete), microseconds since the
    /// Unix epoch.
    pub updated_us: i64,
    /// The store's revision at this row's last change. 0 on a device's
    /// own not-yet-registered row.
    pub rev: u64,
    /// A tombstone: kept so an offline device learns of the delete.
    pub deleted: bool,
    /// What a replica with no history in common with this one meets
    /// ([`Lineage`]). Set by the first registration.
    pub lineage: Lineage,
}

/// A doc to register. The id is the caller's.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct CreateDoc {
    pub doc_id: Uuid,
    pub kind: String,
    pub name: String,
    pub parent: Option<Uuid>,
    /// Set once: a later registration with another policy is
    /// [`DocsError::LineageMismatch`].
    pub lineage: Lineage,
}

/// What a doc's host does with a replica whose history shares nothing
/// with the doc it serves (two copies made apart under one id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, facet::Facet)]
#[repr(u8)]
pub enum Lineage {
    /// Refuse it ([`crate::peer::FOREIGN_LINEAGE`]): the copies are two
    /// documents. Right for a doc whose content would duplicate when two
    /// histories merge (text seeded on both sides, a list built twice),
    /// and for anything forked from another doc's history.
    #[default]
    Strict,
    /// Merge it: Loro joins two independently made docs whose content is
    /// root containers by name (a map keyed by entry, say), field by
    /// field. Right for a doc every device makes under one derived id,
    /// offline, before any of them has synced.
    Merge,
}

impl Lineage {
    /// The stored word: `strict`, `merge`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Merge => "merge",
        }
    }

    /// Parse the stored word; anything unknown is [`Self::Strict`], the
    /// safe answer.
    #[must_use]
    pub fn parse(word: &str) -> Self {
        if word == "merge" {
            Self::Merge
        } else {
            Self::Strict
        }
    }
}

/// Which rows a [`Docs::list`] returns, in revision order.
#[derive(Debug, Clone, Default, PartialEq, Eq, facet::Facet)]
pub struct DocQuery {
    /// Only this kind.
    pub kind: Option<String>,
    /// Only rows changed after this revision (0: everything).
    pub since_rev: u64,
    /// Tombstones too. A device catching up wants them.
    pub include_deleted: bool,
    /// At most this many rows (0: no limit).
    pub limit: u32,
}

/// One page of a [`Docs::list`].
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct DocPage {
    pub docs: Vec<DocMeta>,
    /// Pass as `since_rev` to continue: the last row's revision when
    /// `more`, the store's head otherwise.
    pub cursor: u64,
    /// The page stopped at `limit`; there are more rows after `cursor`.
    pub more: bool,
}

/// A frame of the [`Docs::changes`] stream.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum DocChange {
    /// The first frame: the subscription is live, and the store's head
    /// was `head_rev` when it attached. List from your cursor now.
    Attached {
        head_rev: u64,
    },
    Created(DocMeta),
    Renamed(DocMeta),
    Deleted(DocMeta),
}

impl DocChange {
    /// The row this change carries (none for [`Self::Attached`]).
    #[must_use]
    pub const fn meta(&self) -> Option<&DocMeta> {
        match self {
            Self::Attached { .. } => None,
            Self::Created(m) | Self::Renamed(m) | Self::Deleted(m) => Some(m),
        }
    }
}

/// Why a catalog call failed.
#[architect::error]
#[derive(Eq)]
pub enum DocsError {
    #[error("no such doc")]
    NotFound,
    /// The doc was deleted: it cannot be created again, renamed or synced.
    #[error("the doc was deleted")]
    #[architect(http_status = 410)]
    Deleted,
    /// The id is registered with another kind (the held kind).
    #[error("the id is registered as a `{0}`")]
    #[architect(http_status = 409)]
    KindMismatch(String),
    /// The id is registered with another [`Lineage`] (the held one).
    #[error("the id is registered with lineage {0:?}")]
    #[architect(http_status = 409)]
    LineageMismatch(Lineage),
    /// The row moved on (its current revision).
    #[error("the doc changed (now at revision {0})")]
    #[architect(http_status = 409)]
    Stale(u64),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not permitted")]
    Forbidden,
    #[error("internal error: {0}")]
    Internal(String),
}

/// The catalog lane. Content syncs over
/// [`DocSync`](crate::sync::DocSync), under the same id.
#[architect::rpc]
pub trait Docs {
    /// Register a doc. Idempotent on `doc_id`: registering an id again
    /// with the same kind returns the row as it is (the first name and
    /// parent stand); another kind is [`DocsError::KindMismatch`], a
    /// deleted id [`DocsError::Deleted`].
    async fn create(&self, req: CreateDoc) -> Result<DocMeta, DocsError>;

    /// One row, tombstones included.
    async fn get(&self, doc_id: Uuid) -> Result<DocMeta, DocsError>;

    /// Rows in revision order (see [`DocQuery`], [`DocPage`]).
    async fn list(&self, query: DocQuery) -> Result<DocPage, DocsError>;

    /// Rename. `expected_rev`, when given, must be the row's current
    /// revision or the call is [`DocsError::Stale`].
    async fn rename(
        &self,
        doc_id: Uuid,
        name: String,
        expected_rev: Option<u64>,
    ) -> Result<DocMeta, DocsError>;

    /// Tombstone the doc. Idempotent. Its bytes are kept.
    async fn delete(&self, doc_id: Uuid) -> Result<DocMeta, DocsError>;

    /// Every catalog change, after a first [`DocChange::Attached`].
    #[subscribe]
    fn changes(&self) -> DocChange;
}

/// Where a host keeps its catalog. Backends allocate revisions; the
/// host above them decides what a call means.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait DocsStore: MaybeSendSync + 'static {
    /// One row, tombstones included.
    async fn get(&self, doc_id: Uuid) -> Result<Option<DocMeta>, DocsError>;

    /// Rows after `query.since_rev`, in revision order, filtered and
    /// paged as [`DocQuery`] says; `cursor` as [`DocPage`] says.
    async fn list(&self, query: &DocQuery) -> Result<DocPage, DocsError>;

    /// Store `row` under the next revision (head + 1, returned in the
    /// stored row), if its stored revision is still `expect` (`None`:
    /// no row yet). Otherwise [`DocsError::Stale`] with the current one
    /// (0 when an `expect`ed row is missing).
    async fn put(&self, row: DocMeta, expect: Option<u64>) -> Result<DocMeta, DocsError>;

    /// The newest revision (0 for an empty store).
    async fn head(&self) -> Result<u64, DocsError>;
}

/// Check a `kind` for shape: 1 to [`MAX_KIND_LEN`] of `[a-z0-9._-]`.
pub fn check_kind(kind: &str) -> Result<(), DocsError> {
    let ok = !kind.is_empty()
        && kind.len() <= MAX_KIND_LEN
        && kind.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        });
    if ok {
        Ok(())
    } else {
        Err(DocsError::Invalid(format!(
            "kind `{kind}`: 1 to {MAX_KIND_LEN} of [a-z0-9._-]"
        )))
    }
}

/// Check a `name`: at most [`MAX_NAME_LEN`] characters.
pub fn check_name(name: &str) -> Result<(), DocsError> {
    if name.chars().count() > MAX_NAME_LEN {
        return Err(DocsError::Invalid(format!(
            "name longer than {MAX_NAME_LEN} characters"
        )));
    }
    Ok(())
}

/// Check a whole [`CreateDoc`].
pub fn check_create(req: &CreateDoc) -> Result<(), DocsError> {
    if req.doc_id.is_nil() {
        return Err(DocsError::Invalid("nil doc id".into()));
    }
    if req.parent == Some(req.doc_id) {
        return Err(DocsError::Invalid("a doc cannot be its own parent".into()));
    }
    check_kind(&req.kind)?;
    check_name(&req.name)
}

/// Now, in microseconds since the Unix epoch.
#[must_use]
pub fn now_us() -> i64 {
    chrono::Utc::now().timestamp_micros()
}

/// Order `docs` so every doc comes after its parent when the parent is
/// among them too (a branch made offline from a project also made
/// offline). Otherwise stable.
#[must_use]
pub fn parent_first<T>(mut docs: Vec<T>, id: impl Fn(&T) -> (Uuid, Option<Uuid>)) -> Vec<T> {
    let mut out = Vec::with_capacity(docs.len());
    let mut placed = std::collections::HashSet::new();
    while !docs.is_empty() {
        let pending: std::collections::HashSet<Uuid> = docs.iter().map(|d| id(d).0).collect();
        let before = docs.len();
        let mut rest = Vec::with_capacity(docs.len());
        for d in docs {
            let (doc, parent) = id(&d);
            let waits = parent.is_some_and(|p| pending.contains(&p) && !placed.contains(&p));
            if waits {
                rest.push(d);
            } else {
                placed.insert(doc);
                out.push(d);
            }
        }
        // A cycle (it cannot come from `create`, which refuses a doc as
        // its own parent, but rows are data): place the rest as they are.
        if rest.len() == before {
            out.extend(rest);
            break;
        }
        docs = rest;
    }
    out
}

/// A catalog in memory: tests, demos, a node that keeps nothing.
#[derive(Clone, Default)]
pub struct InMemoryDocsStore {
    inner: std::sync::Arc<std::sync::Mutex<Rows>>,
}

#[derive(Default)]
struct Rows {
    head: u64,
    rows: std::collections::HashMap<Uuid, DocMeta>,
}

impl InMemoryDocsStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Whether `row` passes `query`'s filters (revision aside).
#[must_use]
pub fn matches(query: &DocQuery, row: &DocMeta) -> bool {
    (query.include_deleted || !row.deleted) && query.kind.as_ref().is_none_or(|k| *k == row.kind)
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl DocsStore for InMemoryDocsStore {
    async fn get(&self, doc_id: Uuid) -> Result<Option<DocMeta>, DocsError> {
        Ok(architect::lock(&self.inner).rows.get(&doc_id).cloned())
    }

    async fn list(&self, query: &DocQuery) -> Result<DocPage, DocsError> {
        let guard = architect::lock(&self.inner);
        let mut rows: Vec<&DocMeta> = guard
            .rows
            .values()
            .filter(|r| r.rev > query.since_rev)
            .collect();
        rows.sort_by_key(|r| r.rev);
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
        let mut docs = Vec::new();
        for row in rows.into_iter().filter(|r| matches(query, r)) {
            if limit != 0 && docs.len() == limit {
                let cursor = docs.last().map_or(query.since_rev, |d: &DocMeta| d.rev);
                return Ok(DocPage {
                    docs,
                    cursor,
                    more: true,
                });
            }
            docs.push(row.clone());
        }
        let cursor = guard.head.max(query.since_rev);
        drop(guard);
        Ok(DocPage {
            docs,
            cursor,
            more: false,
        })
    }

    async fn put(&self, mut row: DocMeta, expect: Option<u64>) -> Result<DocMeta, DocsError> {
        let mut guard = architect::lock(&self.inner);
        let current = guard.rows.get(&row.doc_id).map(|r| r.rev);
        if current != expect {
            return Err(DocsError::Stale(current.unwrap_or(0)));
        }
        guard.head = guard.head.saturating_add(1);
        row.rev = guard.head;
        guard.rows.insert(row.doc_id, row.clone());
        drop(guard);
        Ok(row)
    }

    async fn head(&self) -> Result<u64, DocsError> {
        Ok(architect::lock(&self.inner).head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_shape_checked_only() {
        assert!(check_kind("session").is_ok());
        assert!(check_kind("studio.rig-v2_b").is_ok());
        assert!(check_kind("").is_err());
        assert!(check_kind("Session").is_err());
        assert!(check_kind("a/b").is_err());
        assert!(check_kind(&"x".repeat(MAX_KIND_LEN + 1)).is_err());
    }

    #[test]
    fn parents_come_first() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        // c forks b, b forks a; given children first.
        let docs = vec![(c, Some(b)), (b, Some(a)), (a, None)];
        let order: Vec<Uuid> = parent_first(docs, |d| *d)
            .into_iter()
            .map(|d| d.0)
            .collect();
        assert_eq!(order, vec![a, b, c]);
        // A parent that is not pending does not hold its child back.
        let outside = Uuid::new_v4();
        let order = parent_first(vec![(c, Some(outside))], |d| *d);
        assert_eq!(order.len(), 1);
    }

    fn row(kind: &str) -> DocMeta {
        DocMeta {
            doc_id: Uuid::new_v4(),
            kind: kind.into(),
            name: String::new(),
            parent: None,
            created_by: String::new(),
            origin: None,
            created_us: 0,
            updated_us: 0,
            rev: 0,
            deleted: false,
            lineage: Lineage::Strict,
        }
    }

    #[tokio::test]
    async fn in_memory_store_pages_by_revision() {
        let store = InMemoryDocsStore::new();
        let a = store.put(row("session"), None).await.unwrap();
        let b = store.put(row("setlist"), None).await.unwrap();
        let c = store.put(row("session"), None).await.unwrap();
        assert_eq!((a.rev, b.rev, c.rev), (1, 2, 3));
        assert_eq!(
            store.put(a.clone(), None).await.unwrap_err(),
            DocsError::Stale(1),
            "a second insert of one id is stale"
        );

        let page = store
            .list(&DocQuery {
                limit: 2,
                ..DocQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(page.docs.len(), 2);
        assert!(page.more);
        let rest = store
            .list(&DocQuery {
                since_rev: page.cursor,
                ..DocQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(rest.docs, vec![c.clone()]);
        assert_eq!((rest.cursor, rest.more), (3, false));

        let sessions = store
            .list(&DocQuery {
                kind: Some("session".into()),
                ..DocQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(sessions.docs.len(), 2);
        assert_eq!(sessions.cursor, 3, "the cursor is the head, filter or not");
    }
}
