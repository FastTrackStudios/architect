//! [`DocsDriver`]: a device's documents, kept in step with a host.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::{
    CreateDoc, DocChange, DocMeta, DocQuery, DocsClient, DocsError, DocsStreamClient, check_create,
    now_us, parent_first,
};
use crate::sync::{DocSyncClient, SyncedDoc};
use crate::{CrdtDoc, PersistError, Persistence};

/// One doc as a device holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDoc {
    /// The host's row once registered; the device's own until then
    /// (`rev` 0, `created_by` empty).
    pub meta: DocMeta,
    /// Made here and not registered yet.
    pub pending: bool,
}

/// The device's own catalog: the docs it holds, which of them still
/// wait to be registered, and how far it has read the host's catalog.
#[async_trait::async_trait]
pub trait DocsIndex: Send + Sync + 'static {
    async fn docs(&self) -> Result<Vec<LocalDoc>, DocsError>;
    async fn get(&self, doc_id: Uuid) -> Result<Option<LocalDoc>, DocsError>;
    async fn upsert(&self, doc: &LocalDoc) -> Result<(), DocsError>;
    /// The host revision read up to (0 before the first catch-up).
    async fn cursor(&self) -> Result<u64, DocsError>;
    async fn set_cursor(&self, rev: u64) -> Result<(), DocsError>;
}

/// Where each doc's bytes live on the device.
///
/// Asked once per doc per process, with the doc's row: for a doc made
/// here, and for one this device first hears of from the host (made on
/// another device), which is the moment to create its local storage (a
/// project file, say). Rows reach it parent first, when the parent is
/// one this device is learning of too.
#[async_trait::async_trait]
pub trait DocStorage: Send + Sync + 'static {
    async fn persistence(&self, meta: &DocMeta) -> Result<Arc<dyn Persistence>, PersistError>;
}

/// Every doc in one store (a device database holding them all).
#[derive(Clone)]
pub struct SharedStorage(pub Arc<dyn Persistence>);

#[async_trait::async_trait]
impl DocStorage for SharedStorage {
    async fn persistence(&self, _meta: &DocMeta) -> Result<Arc<dyn Persistence>, PersistError> {
        Ok(self.0.clone())
    }
}

/// A [`DocsIndex`] in memory: tests, and a device that keeps nothing.
#[derive(Clone, Default)]
pub struct InMemoryDocsIndex {
    inner: Arc<std::sync::Mutex<(u64, HashMap<Uuid, LocalDoc>)>>,
}

impl InMemoryDocsIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl DocsIndex for InMemoryDocsIndex {
    async fn docs(&self) -> Result<Vec<LocalDoc>, DocsError> {
        Ok(architect::lock(&self.inner).1.values().cloned().collect())
    }
    async fn get(&self, doc_id: Uuid) -> Result<Option<LocalDoc>, DocsError> {
        Ok(architect::lock(&self.inner).1.get(&doc_id).cloned())
    }
    async fn upsert(&self, doc: &LocalDoc) -> Result<(), DocsError> {
        architect::lock(&self.inner)
            .1
            .insert(doc.meta.doc_id, doc.clone());
        Ok(())
    }
    async fn cursor(&self) -> Result<u64, DocsError> {
        Ok(architect::lock(&self.inner).0)
    }
    async fn set_cursor(&self, rev: u64) -> Result<(), DocsError> {
        architect::lock(&self.inner).0 = rev;
        Ok(())
    }
}

/// The three clients one connection to a host gives a driver.
#[derive(Clone)]
pub struct DocsLane {
    pub docs: DocsClient,
    pub changes: DocsStreamClient,
    pub sync: DocSyncClient,
}

/// One open doc: the replica, and its sync wiring (made with it, so
/// every local edit lands in the outbox).
struct Slot {
    doc: CrdtDoc,
    synced: tokio::sync::Mutex<SyncedDoc>,
}

struct Inner {
    index: Arc<dyn DocsIndex>,
    storage: Arc<dyn DocStorage>,
    slots: tokio::sync::Mutex<HashMap<Uuid, Arc<Slot>>>,
    /// Docs created while a session runs, for it to register.
    kick: tokio::sync::mpsc::UnboundedSender<Uuid>,
    /// Held by the running session: one at a time.
    session: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Uuid>>,
    events: tokio::sync::broadcast::Sender<DocChange>,
}

/// A device's docs for one owner (a person's own space, or one org),
/// kept in step with that owner's host.
///
/// Docs are created here whether or not the device is online, and
/// opened only through the driver ([`Self::create`], [`Self::open`]),
/// which wires each replica for sync before anything can edit it.
/// [`Self::run`] is one connected session: it registers what was made
/// offline (parent first; a doc syncs only once registered), catches
/// up the catalog (adopting docs made elsewhere through
/// [`DocStorage`]), then syncs every live doc **durably** (what
/// arrives is written to the doc's persistence, so a restart while
/// offline keeps it) and follows the catalog until the connection
/// drops. Run it again on reconnect: everything is a delta.
///
/// A device with several owners runs a driver per owner, each with its
/// own index, storage and lane.
#[derive(Clone)]
pub struct DocsDriver {
    inner: Arc<Inner>,
}

/// The first wait before retrying one doc's session, and the longest.
const RETRY_FLOOR: Duration = Duration::from_millis(250);
const RETRY_CAP: Duration = Duration::from_secs(30);

fn persist(e: &PersistError) -> DocsError {
    DocsError::Internal(e.to_string())
}

// By value: an error-mapping adapter for `map_err`.
#[allow(clippy::needless_pass_by_value)]
fn transport(e: impl std::fmt::Display) -> DocsError {
    DocsError::Transport(e.to_string())
}

impl DocsDriver {
    pub fn new(index: impl DocsIndex, storage: impl DocStorage) -> Self {
        let (kick, rx) = tokio::sync::mpsc::unbounded_channel();
        let (events, _) = tokio::sync::broadcast::channel(256);
        Self {
            inner: Arc::new(Inner {
                index: Arc::new(index),
                storage: Arc::new(storage),
                slots: tokio::sync::Mutex::new(HashMap::new()),
                kick,
                session: tokio::sync::Mutex::new(rx),
                events,
            }),
        }
    }

    /// Every doc the device holds, tombstones included.
    pub async fn docs(&self) -> Result<Vec<LocalDoc>, DocsError> {
        self.inner.index.docs().await
    }

    /// What changes in the device's catalog: docs made here
    /// ([`DocChange::Created`] with `rev` 0) and every host row applied.
    #[must_use]
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<DocChange> {
        self.inner.events.subscribe()
    }

    /// Make a doc on this device, online or not. Idempotent on the id
    /// like the host's `create`.
    pub async fn create(&self, req: CreateDoc) -> Result<CrdtDoc, DocsError> {
        check_create(&req)?;
        if let Some(held) = self.inner.index.get(req.doc_id).await? {
            if held.meta.kind != req.kind {
                return Err(DocsError::KindMismatch(held.meta.kind));
            }
            return self.open(req.doc_id).await;
        }
        let now = now_us();
        let meta = DocMeta {
            doc_id: req.doc_id,
            kind: req.kind,
            name: req.name,
            parent: req.parent,
            created_by: String::new(),
            origin: None,
            created_us: now,
            updated_us: now,
            rev: 0,
            deleted: false,
        };
        self.inner
            .index
            .upsert(&LocalDoc {
                meta: meta.clone(),
                pending: true,
            })
            .await?;
        let slot = self.slot(&meta).await?;
        let _ = self.inner.events.send(DocChange::Created(meta));
        // A running session registers it now; otherwise the next does.
        let _ = self.inner.kick.send(req.doc_id);
        Ok(slot.doc.clone())
    }

    /// A doc the device holds, opened (once per process) for reading
    /// and editing.
    pub async fn open(&self, doc_id: Uuid) -> Result<CrdtDoc, DocsError> {
        let held = self
            .inner
            .index
            .get(doc_id)
            .await?
            .ok_or(DocsError::NotFound)?;
        if held.meta.deleted {
            return Err(DocsError::Deleted);
        }
        Ok(self.slot(&held.meta).await?.doc.clone())
    }

    async fn slot(&self, meta: &DocMeta) -> Result<Arc<Slot>, DocsError> {
        let mut slots = self.inner.slots.lock().await;
        if let Some(slot) = slots.get(&meta.doc_id) {
            return Ok(slot.clone());
        }
        let persistence = self
            .inner
            .storage
            .persistence(meta)
            .await
            .map_err(|e| persist(&e))?;
        let doc = CrdtDoc::open_shared(meta.doc_id, persistence)
            .await
            .map_err(|e| persist(&e))?;
        let synced = SyncedDoc::new(meta.doc_id, doc.clone()).durable();
        let slot = Arc::new(Slot {
            doc,
            synced: tokio::sync::Mutex::new(synced),
        });
        slots.insert(meta.doc_id, slot.clone());
        drop(slots);
        Ok(slot)
    }

    /// One connected session, until the connection drops (or the
    /// returned future is dropped). `Err` for a transport failure or a
    /// session already running; run it again on reconnect.
    pub async fn run(&self, lane: &DocsLane) -> Result<(), DocsError> {
        let Ok(mut kicks) = self.inner.session.try_lock() else {
            return Err(DocsError::Internal("a session is already running".into()));
        };
        let mut session = Session {
            driver: self,
            lane,
            syncing: tokio::task::JoinSet::new(),
            running: HashMap::new(),
        };

        // Subscribe first, so nothing published from here on is missed.
        let (tx, mut rx) = vox::channel::<DocChange>();
        let mut subscription = std::pin::pin!(lane.changes.changes(tx));
        let mut attached = false;
        while !attached {
            tokio::select! {
                res = &mut subscription => return session.ended(res.map_err(transport)),
                frame = rx.recv() => match frame {
                    Ok(Some(frame)) => {
                        let mut owned = None;
                        let _ = frame.map(|f| owned = Some(f));
                        attached = matches!(owned, Some(DocChange::Attached { .. }));
                    }
                    _ => return session.ended(Ok(())),
                },
            }
        }
        // Drain what was created before the session: `register_pending`
        // reads the index, which already holds them.
        while kicks.try_recv().is_ok() {}
        session.register_pending().await?;
        session.catch_up().await?;
        session.start_all().await?;

        loop {
            tokio::select! {
                res = &mut subscription => return session.ended(res.map_err(transport)),
                frame = rx.recv() => {
                    let mut owned = None;
                    match frame {
                        Ok(Some(frame)) => {
                            let _ = frame.map(|f| owned = Some(f));
                        }
                        _ => return session.ended(Ok(())),
                    }
                    if let Some(change) = owned {
                        session.on_change(change).await?;
                    }
                }
                Some(_) = kicks.recv() => {
                    while kicks.try_recv().is_ok() {}
                    session.register_pending().await?;
                    session.start_all().await?;
                }
            }
        }
    }
}

/// One [`DocsDriver::run`].
struct Session<'a> {
    driver: &'a DocsDriver,
    lane: &'a DocsLane,
    /// Each live doc's sync loop; dropped (aborted) with the session.
    syncing: tokio::task::JoinSet<()>,
    running: HashMap<Uuid, tokio::task::AbortHandle>,
}

impl Session<'_> {
    fn index(&self) -> &dyn DocsIndex {
        &*self.driver.inner.index
    }

    fn ended(&mut self, res: Result<(), DocsError>) -> Result<(), DocsError> {
        self.syncing.abort_all();
        res
    }

    /// Register every doc made here, parent first.
    async fn register_pending(&self) -> Result<(), DocsError> {
        let pending: Vec<LocalDoc> = self
            .index()
            .docs()
            .await?
            .into_iter()
            .filter(|d| d.pending && !d.meta.deleted)
            .collect();
        for local in parent_first(pending, |d| (d.meta.doc_id, d.meta.parent)) {
            let m = &local.meta;
            let req = CreateDoc {
                doc_id: m.doc_id,
                kind: m.kind.clone(),
                name: m.name.clone(),
                parent: m.parent,
            };
            match self
                .lane
                .docs
                .create(req)
                .await
                .map_err(architect::vox_into_app)
            {
                Ok(meta) => {
                    self.index()
                        .upsert(&LocalDoc {
                            meta,
                            pending: false,
                        })
                        .await?;
                }
                Err(DocsError::Deleted) => {
                    self.index()
                        .upsert(&LocalDoc {
                            meta: DocMeta {
                                deleted: true,
                                ..local.meta.clone()
                            },
                            pending: false,
                        })
                        .await?;
                }
                Err(e @ DocsError::Transport(_)) => return Err(e),
                // A kind clash or a refusal: it stays pending (and
                // unsynced) for the app to resolve.
                Err(e) => {
                    tracing::warn!(doc.id = %m.doc_id, doc.kind = %m.kind, error = %e, "docs: registering a doc failed");
                }
            }
        }
        Ok(())
    }

    /// Read the host's catalog from the device's cursor and apply it.
    async fn catch_up(&mut self) -> Result<(), DocsError> {
        let since_rev = self.index().cursor().await?;
        let page = self
            .lane
            .docs
            .list(DocQuery {
                kind: None,
                since_rev,
                include_deleted: true,
                limit: 0,
            })
            .await
            .map_err(architect::vox_into_app)?;
        // Revision order puts a parent renamed after its fork behind it.
        for meta in parent_first(page.docs, |d| (d.doc_id, d.parent)) {
            self.apply(meta).await?;
        }
        self.index().set_cursor(page.cursor).await
    }

    /// Take one host row into the device's catalog.
    async fn apply(&mut self, meta: DocMeta) -> Result<(), DocsError> {
        let held = self.index().get(meta.doc_id).await?;
        let change = match &held {
            None => DocChange::Created(meta.clone()),
            Some(_) if meta.deleted => DocChange::Deleted(meta.clone()),
            Some(_) => DocChange::Renamed(meta.clone()),
        };
        if held.as_ref().is_some_and(|h| h.meta == meta && !h.pending) {
            return Ok(());
        }
        self.index()
            .upsert(&LocalDoc {
                meta: meta.clone(),
                pending: false,
            })
            .await?;
        if meta.deleted {
            if let Some(task) = self.running.remove(&meta.doc_id) {
                task.abort();
            }
        }
        let _ = self.driver.inner.events.send(change);
        Ok(())
    }

    /// A frame of the host's `changes`.
    async fn on_change(&mut self, change: DocChange) -> Result<(), DocsError> {
        let Some(meta) = change.meta().cloned() else {
            return Ok(());
        };
        let cursor = self.index().cursor().await?;
        if meta.rev <= cursor {
            return Ok(());
        }
        let next = cursor.saturating_add(1);
        if meta.rev > next {
            // The stream dropped something: read the catalog instead.
            self.catch_up().await?;
        } else {
            self.apply(meta).await?;
            self.index().set_cursor(next).await?;
        }
        self.start_all().await
    }

    /// Start syncing every registered, live doc not already syncing.
    async fn start_all(&mut self) -> Result<(), DocsError> {
        let docs = self.index().docs().await?;
        let ready: Vec<LocalDoc> = docs
            .into_iter()
            .filter(|d| !d.pending && !d.meta.deleted && !self.running.contains_key(&d.meta.doc_id))
            .collect();
        for local in parent_first(ready, |d| (d.meta.doc_id, d.meta.parent)) {
            let slot = match self.driver.slot(&local.meta).await {
                Ok(slot) => slot,
                Err(e) => {
                    tracing::warn!(doc.id = %local.meta.doc_id, error = %e, "docs: no local storage for a doc");
                    continue;
                }
            };
            let client = self.lane.sync.clone();
            let doc_id = local.meta.doc_id;
            let task = self.syncing.spawn(keep_synced(doc_id, slot, client));
            self.running.insert(doc_id, task);
        }
        Ok(())
    }
}

/// One doc's sync sessions, retried with backoff while the driver's
/// session lasts. A replica the host refuses as another lineage, or no
/// longer serves, stops here.
// The replica's lock is held for the task's life: one session per doc.
#[allow(clippy::significant_drop_tightening)]
async fn keep_synced(doc_id: Uuid, slot: Arc<Slot>, client: DocSyncClient) {
    let mut synced = slot.synced.lock().await;
    let mut wait = RETRY_FLOOR;
    loop {
        match synced.run(&client).await {
            Ok(()) => wait = RETRY_FLOOR,
            Err(e) => {
                let why = e.to_string();
                if why.contains(crate::peer::FOREIGN_LINEAGE) || why.contains("unknown doc") {
                    tracing::warn!(doc.id = %doc_id, error = %why, "docs: the host will not sync this doc");
                    return;
                }
                tracing::debug!(doc.id = %doc_id, error = %why, "docs: doc session ended; retrying");
            }
        }
        tokio::time::sleep(wait).await;
        wait = wait.saturating_mul(2).min(RETRY_CAP);
    }
}
