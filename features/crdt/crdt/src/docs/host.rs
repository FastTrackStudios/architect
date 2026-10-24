//! [`DocsHost`]: a catalog served, with its documents' content lane.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use architect::PubSub;
use architect_permissions::Principal;
use uuid::Uuid;

use super::{
    CreateDoc, DocChange, DocMeta, DocPage, DocQuery, Docs, DocsError, DocsStore, DocsStreamSource,
    check_create, check_name, now_us,
};
use crate::registry::DocRegistry;
use crate::sync::{DocPresence, DocSync, SyncDown, SyncError};

/// What a caller asks to do to a doc, for [`DocsHost::with_authorizer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocAccess {
    /// See it in the catalog, watch its presence.
    Read,
    /// Register, rename, delete, sync its content.
    Write,
}

type Authorizer = dyn Fn(Option<&Principal>, Uuid, DocAccess) -> bool + Send + Sync;

/// How many times a write retries a revision race before giving up.
const PUT_ATTEMPTS: usize = 8;

/// A catalog ([`Docs`] and its `changes` stream) and the content of its
/// documents ([`DocSync`], [`DocPresence`]) behind one value.
///
/// Mount it for all three services. The content lane admits an id only
/// while the catalog holds it, not deleted (anything else answers
/// [`SyncError::UnknownDoc`], like the registry), refuses a replica from
/// another lineage ([`crate::peer::FOREIGN_LINEAGE`]), and asks the
/// authorizer first. The host never writes into a document: a doc made
/// offline on a device must meet an empty one here, or it would be
/// refused as foreign.
///
/// The `changes` stream is one hub for every subscriber, so what it
/// carries is not filtered per caller: mount a host per audience (Task
/// mounts one per org, whose members all see its catalog).
#[derive(Clone)]
pub struct DocsHost {
    store: Arc<dyn DocsStore>,
    registry: DocRegistry,
    /// Each catalogued id: the revision of the last row applied here and
    /// whether that row is a tombstone. Ordered by revision, so a commit
    /// that reaches the lock late cannot undo a newer one (a rename
    /// re-admitting a doc deleted after it).
    live: Arc<RwLock<HashMap<Uuid, (u64, bool)>>>,
    head: Arc<AtomicU64>,
    hub: PubSub<DocChange>,
    authorize: Arc<Authorizer>,
}

impl DocsHost {
    /// Serve `store`'s catalog and the docs `registry` opens. Reads the
    /// catalog once, to know which ids are live.
    pub async fn open(store: impl DocsStore, registry: DocRegistry) -> Result<Self, DocsError> {
        let store: Arc<dyn DocsStore> = Arc::new(store);
        let mut live = HashMap::new();
        let mut since_rev = 0;
        loop {
            let page = store
                .list(&DocQuery {
                    since_rev,
                    include_deleted: true,
                    limit: 1024,
                    ..DocQuery::default()
                })
                .await?;
            live.extend(page.docs.iter().map(|d| (d.doc_id, (d.rev, d.deleted))));
            since_rev = page.cursor;
            if !page.more {
                break;
            }
        }
        let head = store.head().await?;
        Ok(Self {
            store,
            registry,
            live: Arc::new(RwLock::new(live)),
            head: Arc::new(AtomicU64::new(head)),
            hub: PubSub::sliding(1024),
            authorize: Arc::new(|_, _, _| true),
        })
    }

    /// Decide per call whether the caller (the principal the router's
    /// permissions gate resolved, `None` without one) may do `access` to
    /// a doc. Runs on every catalog call (each row of a `list`) and
    /// every content attach. Default: allow, leaving it to the gate in
    /// front of the lane.
    #[must_use]
    pub fn with_authorizer<A>(mut self, authorize: A) -> Self
    where
        A: Fn(Option<&Principal>, Uuid, DocAccess) -> bool + Send + Sync + 'static,
    {
        self.authorize = Arc::new(authorize);
        self
    }

    /// The registry the content lane serves from.
    #[must_use]
    pub const fn registry(&self) -> &DocRegistry {
        &self.registry
    }

    /// Whether `doc_id` is a live (catalogued, not deleted) doc of this
    /// host: what a router uses to send a `DocSync` here.
    #[must_use]
    pub fn admits(&self, doc_id: Uuid) -> bool {
        matches!(architect::read(&self.live).get(&doc_id), Some((_, false)))
    }

    fn allowed(&self, doc_id: Uuid, access: DocAccess) -> bool {
        let who = architect::permissions_gate::caller();
        (self.authorize)(who.as_ref(), doc_id, access)
    }

    fn check(&self, doc_id: Uuid, access: DocAccess) -> Result<(), DocsError> {
        if self.allowed(doc_id, access) {
            Ok(())
        } else {
            tracing::debug!(doc.id = %doc_id, ?access, "docs: refused by the authorizer");
            Err(DocsError::Forbidden)
        }
    }

    /// Store a row and tell everyone.
    async fn commit(
        &self,
        row: DocMeta,
        expect: Option<u64>,
        change: fn(DocMeta) -> DocChange,
    ) -> Result<DocMeta, DocsError> {
        let stored = self.store.put(row, expect).await?;
        {
            let mut live = architect::write(&self.live);
            let newer = live
                .get(&stored.doc_id)
                .is_none_or(|(rev, _)| *rev < stored.rev);
            if newer {
                live.insert(stored.doc_id, (stored.rev, stored.deleted));
            }
        }
        self.head.fetch_max(stored.rev, Ordering::SeqCst);
        self.hub.publish(change(stored.clone()));
        Ok(stored)
    }

    /// Apply `edit` to the stored row, retrying a revision race. `edit`
    /// answers `None` when there is nothing to change.
    async fn update(
        &self,
        doc_id: Uuid,
        change: fn(DocMeta) -> DocChange,
        edit: impl Fn(&DocMeta) -> Result<Option<DocMeta>, DocsError>,
    ) -> Result<DocMeta, DocsError> {
        for _ in 0..PUT_ATTEMPTS {
            let current = self.store.get(doc_id).await?.ok_or(DocsError::NotFound)?;
            let Some(next) = edit(&current)? else {
                return Ok(current);
            };
            match self.commit(next, Some(current.rev), change).await {
                Err(DocsError::Stale(_)) => {}
                done => return done,
            }
        }
        Err(DocsError::Internal("the row kept changing".into()))
    }
}

/// Who registered a doc, from the caller.
fn creator(who: Option<&Principal>) -> (String, Option<String>) {
    match who {
        Some(Principal::User { user_id }) => (user_id.clone(), None),
        Some(Principal::Host { endpoint }) => (format!("host:{endpoint}"), Some(endpoint.clone())),
        Some(other) => (other.describe(), None),
        None => (String::new(), None),
    }
}

impl Docs for DocsHost {
    async fn create(&self, req: CreateDoc) -> Result<DocMeta, DocsError> {
        check_create(&req)?;
        self.check(req.doc_id, DocAccess::Write)?;
        for _ in 0..PUT_ATTEMPTS {
            if let Some(held) = self.store.get(req.doc_id).await? {
                if held.kind != req.kind {
                    return Err(DocsError::KindMismatch(held.kind));
                }
                if held.deleted {
                    return Err(DocsError::Deleted);
                }
                return Ok(held);
            }
            let who = architect::permissions_gate::caller();
            let (created_by, origin) = creator(who.as_ref());
            let now = now_us();
            let row = DocMeta {
                doc_id: req.doc_id,
                kind: req.kind.clone(),
                name: req.name.clone(),
                parent: req.parent,
                created_by,
                origin,
                created_us: now,
                updated_us: now,
                rev: 0,
                deleted: false,
            };
            match self.commit(row, None, DocChange::Created).await {
                // Another create of this id won the race: answer as it.
                Err(DocsError::Stale(_)) => {}
                done => return done,
            }
        }
        Err(DocsError::Internal("the row kept changing".into()))
    }

    async fn get(&self, doc_id: Uuid) -> Result<DocMeta, DocsError> {
        self.check(doc_id, DocAccess::Read)?;
        self.store.get(doc_id).await?.ok_or(DocsError::NotFound)
    }

    async fn list(&self, query: DocQuery) -> Result<DocPage, DocsError> {
        let mut page = self.store.list(&query).await?;
        page.docs
            .retain(|d| self.allowed(d.doc_id, DocAccess::Read));
        Ok(page)
    }

    async fn rename(
        &self,
        doc_id: Uuid,
        name: String,
        expected_rev: Option<u64>,
    ) -> Result<DocMeta, DocsError> {
        check_name(&name)?;
        self.check(doc_id, DocAccess::Write)?;
        self.update(doc_id, DocChange::Renamed, |current| {
            if current.deleted {
                return Err(DocsError::Deleted);
            }
            if expected_rev.is_some_and(|r| r != current.rev) {
                return Err(DocsError::Stale(current.rev));
            }
            if current.name == name {
                return Ok(None);
            }
            Ok(Some(DocMeta {
                name: name.clone(),
                updated_us: now_us(),
                ..current.clone()
            }))
        })
        .await
    }

    async fn delete(&self, doc_id: Uuid) -> Result<DocMeta, DocsError> {
        self.check(doc_id, DocAccess::Write)?;
        self.update(doc_id, DocChange::Deleted, |current| {
            if current.deleted {
                return Ok(None);
            }
            Ok(Some(DocMeta {
                deleted: true,
                updated_us: now_us(),
                ..current.clone()
            }))
        })
        .await
    }
}

impl DocsStreamSource for DocsHost {
    fn changes_hub(&self) -> &PubSub<DocChange> {
        &self.hub
    }

    fn changes_intro(&self) -> Option<DocChange> {
        Some(DocChange::Attached {
            head_rev: self.head.load(Ordering::SeqCst),
        })
    }
}

impl DocSync for DocsHost {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError> {
        if !self.admits(doc_id) || !self.allowed(doc_id, DocAccess::Write) {
            tracing::debug!(doc.id = %doc_id, "docs: attach refused");
            return Err(SyncError::UnknownDoc);
        }
        crate::peer::check_lineage(&self.registry, doc_id, &from).await?;
        self.registry.sync(doc_id, from, up, down).await
    }
}

impl DocPresence for DocsHost {
    async fn presence(
        &self,
        doc_id: Uuid,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), SyncError> {
        if !self.admits(doc_id) || !self.allowed(doc_id, DocAccess::Read) {
            return Err(SyncError::UnknownDoc);
        }
        self.registry.presence(doc_id, up, down).await
    }
}
