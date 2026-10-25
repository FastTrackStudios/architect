//! [`DocsDriver`]: a device's documents, kept in step with its hosts.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;

use super::{
    CreateDoc, DocChange, DocMeta, DocQuery, DocsClient, DocsError, DocsStreamClient, check_create,
    now_us, parent_first,
};
use crate::sync::{DocSyncClient, DocSyncHost, SyncedDoc};
use crate::{CrdtDoc, PersistError, Persistence};

/// One doc as a device holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDoc {
    /// The hub's row once registered; the device's own until then
    /// (`rev` 0, `created_by` empty).
    pub meta: DocMeta,
    /// Not registered with the hub yet: made here, or learned from a
    /// peer before the hub.
    pub pending: bool,
    /// This index's own revision of the row, contiguous from 1, bumped
    /// on every write: what the device's own lane ([`DriverHost`](super::DriverHost))
    /// serves as `rev` to its peers. The hub's revision stays in
    /// `meta.rev`.
    pub seq: u64,
}

/// The device's own catalog: the docs it holds, which of them still
/// wait to be registered, and how far it has read each lane's catalog.
#[async_trait::async_trait]
pub trait DocsIndex: Send + Sync + 'static {
    async fn docs(&self) -> Result<Vec<LocalDoc>, DocsError>;
    async fn get(&self, doc_id: Uuid) -> Result<Option<LocalDoc>, DocsError>;
    /// Store `doc` under the next `seq` (the stored row is returned).
    async fn upsert(&self, doc: LocalDoc) -> Result<LocalDoc, DocsError>;
    /// The revision of `lane`'s catalog read up to (0 before the first
    /// catch-up).
    async fn cursor(&self, lane: &str) -> Result<u64, DocsError>;
    async fn set_cursor(&self, lane: &str, rev: u64) -> Result<(), DocsError>;
}

/// Where each doc's bytes live on the device.
///
/// Asked once per doc per process, with the doc's row: for a doc made
/// here, and for one this device first hears of from a host (made on
/// another device), which is the moment to create its local storage (a
/// project file, say). Rows reach it parent first, when the parent is
/// one this device is learning of too.
#[async_trait::async_trait]
pub trait DocStorage: Send + Sync + 'static {
    async fn persistence(&self, meta: &DocMeta) -> Result<Arc<dyn Persistence>, PersistError>;

    /// Drop the doc's local bytes ([`DocsDriver::reset`]). The default
    /// compacts the doc to an empty snapshot, which a backend whose
    /// `compact` replaces the snapshot and prunes the log (every one in
    /// this crate) turns into an empty doc. Override to remove a file.
    async fn discard(&self, meta: &DocMeta) -> Result<(), PersistError> {
        let empty = loro::LoroDoc::new()
            .export(loro::ExportMode::Snapshot)
            .map_err(|e| PersistError::Backend(format!("empty snapshot: {e}")))?;
        self.persistence(meta)
            .await?
            .compact(meta.doc_id, &empty)
            .await
    }
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

#[derive(Default)]
struct IndexRows {
    seq: u64,
    rows: HashMap<Uuid, LocalDoc>,
    cursors: HashMap<String, u64>,
}

/// A [`DocsIndex`] in memory: tests, and a device that keeps nothing.
#[derive(Clone, Default)]
pub struct InMemoryDocsIndex {
    inner: Arc<Mutex<IndexRows>>,
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
        Ok(architect::lock(&self.inner)
            .rows
            .values()
            .cloned()
            .collect())
    }
    async fn get(&self, doc_id: Uuid) -> Result<Option<LocalDoc>, DocsError> {
        Ok(architect::lock(&self.inner).rows.get(&doc_id).cloned())
    }
    async fn upsert(&self, mut doc: LocalDoc) -> Result<LocalDoc, DocsError> {
        let mut inner = architect::lock(&self.inner);
        inner.seq = inner.seq.saturating_add(1);
        doc.seq = inner.seq;
        inner.rows.insert(doc.meta.doc_id, doc.clone());
        drop(inner);
        Ok(doc)
    }
    async fn cursor(&self, lane: &str) -> Result<u64, DocsError> {
        Ok(architect::lock(&self.inner)
            .cursors
            .get(lane)
            .copied()
            .unwrap_or(0))
    }
    async fn set_cursor(&self, lane: &str, rev: u64) -> Result<(), DocsError> {
        architect::lock(&self.inner)
            .cursors
            .insert(lane.to_owned(), rev);
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

/// The lane key [`DocsDriver::run`] uses for the hub.
pub const HUB: &str = "hub";

/// Which way a lane's catalog is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaneKind {
    /// The canonical host (Task): its rows are the truth, it registers.
    Hub,
    /// Another device of the owner, serving its [`DriverHost`](super::DriverHost):
    /// its rows merge in, and registering stays the hub's.
    Peer,
}

/// Why a host would not sync a doc ([`DocSyncEvent::Refused`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The host's copy is another history of the same id, and the doc is
    /// [`Lineage::Strict`](super::Lineage::Strict). [`DocsDriver::reset`]
    /// takes the host's.
    ForeignLineage,
    /// The host does not serve it (deleted there, or not catalogued).
    NotServed,
    Other(String),
}

/// What happens to one doc's sync on one lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocSyncEvent {
    /// A session caught up: the host's history is applied here, and what
    /// it lacked has been sent.
    Attached { doc_id: Uuid, lane: String },
    /// The host refused the doc; this lane stops syncing it.
    Refused {
        doc_id: Uuid,
        lane: String,
        reason: Refusal,
    },
    /// A session ended (the connection dropped); it is retried.
    Detached { doc_id: Uuid, lane: String },
    /// [`DocsDriver::reset`] dropped the local copy; open the doc again.
    Reset { doc_id: Uuid },
}

/// Something a running session must act on.
#[derive(Debug, Clone, Copy)]
enum Kick {
    /// The index changed here (a doc made, adopted or registered with a
    /// peer): register and start what is new.
    Changed,
    /// A doc's local copy was dropped: stop its sync and start again.
    Reset(Uuid),
}

/// Where an update imported on one lane goes next.
enum Sink {
    /// A lane's outbox: sent up like a local edit.
    Outbox(tokio::sync::mpsc::UnboundedSender<Vec<u8>>),
    /// The device's own host for the doc: sent down to its peers.
    Host(DocSyncHost),
}

/// One doc's relay: every update imported on one lane is passed to every
/// other lane (and to the device's own host), never back to where it
/// came from. Imports fire no local-update subscription, so without it
/// an edit from the hub would reach a connected peer only on its next
/// attach.
#[derive(Default)]
pub(super) struct Relay {
    sinks: Mutex<Vec<(String, Sink)>>,
}

/// The lane key the device's own host relays under.
pub(super) const HOST_LANE: &str = "host";

impl Relay {
    fn add(&self, key: &str, sink: Sink) {
        architect::lock(&self.sinks).push((key.to_owned(), sink));
    }

    pub(super) fn forward(&self, from: &str, bytes: &[u8]) {
        for (key, sink) in architect::lock(&self.sinks).iter() {
            if key == from {
                continue;
            }
            match sink {
                Sink::Outbox(tx) => {
                    let _ = tx.send(bytes.to_vec());
                }
                Sink::Host(host) => host.publish(bytes.to_vec()),
            }
        }
    }
}

/// One open doc: the replica, a sync wiring per lane (made once and
/// kept, so its outbox gathers edits while that lane is down), the
/// relay between them and, once a peer attaches, the device's host.
pub(super) struct Slot {
    pub(super) doc: CrdtDoc,
    lanes: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<SyncedDoc>>>>,
    pub(super) relay: Arc<Relay>,
    host: std::sync::OnceLock<DocSyncHost>,
}

impl Slot {
    /// The device's own host for this doc (its peers attach to it).
    pub(super) fn host(&self, doc_id: Uuid) -> DocSyncHost {
        self.host
            .get_or_init(|| {
                let relay = self.relay.clone();
                let host = DocSyncHost::new(doc_id, self.doc.clone()).on_import(Arc::new(
                    move |bytes: &[u8]| relay.forward(HOST_LANE, bytes),
                ));
                self.relay.add(HOST_LANE, Sink::Host(host.clone()));
                host
            })
            .clone()
    }
}

pub(super) struct Inner {
    pub(super) index: Arc<dyn DocsIndex>,
    storage: Arc<dyn DocStorage>,
    slots: tokio::sync::Mutex<HashMap<Uuid, Arc<Slot>>>,
    kicks: tokio::sync::broadcast::Sender<Kick>,
    /// The lanes with a session running (one per key).
    running: Mutex<HashSet<String>>,
    /// Docs [`DocsDriver::reset`] is emptying: not opened meanwhile.
    resetting: Mutex<HashSet<Uuid>>,
    events: tokio::sync::broadcast::Sender<DocChange>,
    sync_events: tokio::sync::broadcast::Sender<DocSyncEvent>,
    /// Every index write, as the device's own lane serves it (`rev` =
    /// `seq`).
    pub(super) host_hub: architect::PubSub<DocChange>,
    pub(super) head_seq: AtomicU64,
}

/// A device's docs for one owner (a person's own space, or one org),
/// kept in step with that owner's hub and, peer to peer, with the
/// owner's other devices.
///
/// Docs are created here whether or not the device is online, and
/// opened only through the driver ([`Self::create`], [`Self::open`]),
/// which wires each replica for sync before anything can edit it.
/// [`Self::run`] is one connected session with the hub: it registers
/// what was made offline (parent first; a doc syncs with the hub only
/// once registered), catches up the catalog (adopting docs made
/// elsewhere through [`DocStorage`]), then syncs every live doc
/// **durably** (what arrives is written to the doc's persistence, so a
/// restart while offline keeps it) and follows the catalog until the
/// connection drops. [`Self::run_peer`] is the same against another
/// device's [`Self::host`]. Sessions run concurrently, one per lane key;
/// what one lane brings in is relayed to the others. Run each again on
/// reconnect: everything is a delta.
///
/// A device with several owners runs a driver per owner, each with its
/// own index, storage and lanes.
#[derive(Clone)]
pub struct DocsDriver {
    pub(super) inner: Arc<Inner>,
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

/// The change a row's arrival is, given what was held before.
const fn change_for(held: Option<&LocalDoc>, meta: DocMeta) -> DocChange {
    match held {
        None => DocChange::Created(meta),
        Some(_) if meta.deleted => DocChange::Deleted(meta),
        Some(_) => DocChange::Renamed(meta),
    }
}

/// A row as the device's own lane serves it: its `rev` is the index's.
pub(super) fn peer_view(local: &LocalDoc) -> DocMeta {
    DocMeta {
        rev: local.seq,
        ..local.meta.clone()
    }
}

/// A peer's row merged into ours: a tombstone wins, the later rename
/// wins, everything else (the hub's revision, the kind, the lineage)
/// stays ours. `None` when nothing changes.
fn merge_peer(ours: &DocMeta, theirs: &DocMeta) -> Option<DocMeta> {
    let mut merged = ours.clone();
    let deleted = theirs.deleted && !ours.deleted;
    let renamed = theirs.updated_us > ours.updated_us && theirs.name != ours.name;
    if !deleted && !renamed {
        return None;
    }
    if deleted {
        merged.deleted = true;
    }
    if renamed {
        merged.name.clone_from(&theirs.name);
    }
    merged.updated_us = ours.updated_us.max(theirs.updated_us);
    Some(merged)
}

impl DocsDriver {
    pub fn new(index: impl DocsIndex, storage: impl DocStorage) -> Self {
        let (kicks, _) = tokio::sync::broadcast::channel(256);
        let (events, _) = tokio::sync::broadcast::channel(256);
        let (sync_events, _) = tokio::sync::broadcast::channel(1024);
        Self {
            inner: Arc::new(Inner {
                index: Arc::new(index),
                storage: Arc::new(storage),
                slots: tokio::sync::Mutex::new(HashMap::new()),
                kicks,
                running: Mutex::new(HashSet::new()),
                resetting: Mutex::new(HashSet::new()),
                events,
                sync_events,
                host_hub: architect::PubSub::sliding(1024),
                head_seq: AtomicU64::new(0),
            }),
        }
    }

    /// Every doc the device holds, tombstones included.
    pub async fn docs(&self) -> Result<Vec<LocalDoc>, DocsError> {
        self.inner.index.docs().await
    }

    /// What changes in the device's catalog: docs made here
    /// ([`DocChange::Created`] with `rev` 0) and every row applied from a
    /// lane.
    #[must_use]
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<DocChange> {
        self.inner.events.subscribe()
    }

    /// What happens to each doc's sync, per lane ([`DocSyncEvent`]).
    #[must_use]
    pub fn sync_events(&self) -> tokio::sync::broadcast::Receiver<DocSyncEvent> {
        self.inner.sync_events.subscribe()
    }

    /// Write a row to the index and tell everyone who follows it.
    pub(super) async fn put(
        &self,
        doc: LocalDoc,
        change: DocChange,
    ) -> Result<LocalDoc, DocsError> {
        let stored = self.inner.index.upsert(doc).await?;
        self.inner.head_seq.fetch_max(stored.seq, Ordering::SeqCst);
        let view = peer_view(&stored);
        let host_change = match change {
            DocChange::Created(_) => DocChange::Created(view),
            DocChange::Deleted(_) => DocChange::Deleted(view),
            _ => DocChange::Renamed(view),
        };
        self.inner.host_hub.publish(host_change);
        let _ = self.inner.events.send(change);
        Ok(stored)
    }

    pub(super) fn kick_changed(&self) {
        let _ = self.inner.kicks.send(Kick::Changed);
    }

    /// Make a doc on this device, online or not. Idempotent on the id
    /// like the host's `create`.
    pub async fn create(&self, req: CreateDoc) -> Result<CrdtDoc, DocsError> {
        check_create(&req)?;
        if let Some(held) = self.inner.index.get(req.doc_id).await? {
            if held.meta.kind != req.kind {
                return Err(DocsError::KindMismatch(held.meta.kind));
            }
            if held.meta.lineage != req.lineage {
                return Err(DocsError::LineageMismatch(held.meta.lineage));
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
            lineage: req.lineage,
        };
        let local = LocalDoc {
            meta: meta.clone(),
            pending: true,
            seq: 0,
        };
        self.put(local, DocChange::Created(meta.clone())).await?;
        let slot = self.slot(&meta).await?;
        // A running session registers it now; otherwise the next does.
        self.kick_changed();
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

    /// Drop the doc's local copy and take the hosts' instead: the way
    /// out of [`Refusal::ForeignLineage`] for a
    /// [`Lineage::Strict`](super::Lineage::Strict) doc.
    /// Edits only this copy held are lost. Its sync stops on every lane,
    /// [`DocStorage::discard`] empties it, and the running sessions
    /// adopt the hosts' history; a handle from before is stale, so open
    /// the doc again after [`DocSyncEvent::Reset`].
    pub async fn reset(&self, doc_id: Uuid) -> Result<(), DocsError> {
        let held = self
            .inner
            .index
            .get(doc_id)
            .await?
            .ok_or(DocsError::NotFound)?;
        // Nothing reopens the doc until its bytes are gone: a session
        // starting it again in between would reload the old history.
        architect::lock(&self.inner.resetting).insert(doc_id);
        self.inner.slots.lock().await.remove(&doc_id);
        let _ = self.inner.kicks.send(Kick::Reset(doc_id));
        let discarded = self.inner.storage.discard(&held.meta).await;
        architect::lock(&self.inner.resetting).remove(&doc_id);
        discarded.map_err(|e| persist(&e))?;
        let _ = self.inner.sync_events.send(DocSyncEvent::Reset { doc_id });
        let _ = self.inner.kicks.send(Kick::Changed);
        Ok(())
    }

    pub(super) async fn slot(&self, meta: &DocMeta) -> Result<Arc<Slot>, DocsError> {
        if architect::lock(&self.inner.resetting).contains(&meta.doc_id) {
            return Err(DocsError::Internal("the doc is being reset".into()));
        }
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
        let slot = Arc::new(Slot {
            doc,
            lanes: tokio::sync::Mutex::new(HashMap::new()),
            relay: Arc::new(Relay::default()),
            host: std::sync::OnceLock::new(),
        });
        slots.insert(meta.doc_id, slot.clone());
        drop(slots);
        Ok(slot)
    }

    /// The doc's sync wiring for `lane`, made on first use and kept.
    async fn lane_replica(
        &self,
        slot: &Arc<Slot>,
        doc_id: Uuid,
        lane: &str,
    ) -> Arc<tokio::sync::Mutex<SyncedDoc>> {
        let mut lanes = slot.lanes.lock().await;
        if let Some(synced) = lanes.get(lane) {
            return synced.clone();
        }
        let relay = slot.relay.clone();
        let from = lane.to_owned();
        let events = self.inner.sync_events.clone();
        let attached = DocSyncEvent::Attached {
            doc_id,
            lane: lane.to_owned(),
        };
        let synced = SyncedDoc::new(doc_id, slot.doc.clone())
            .durable()
            .on_import(Arc::new(move |bytes: &[u8]| relay.forward(&from, bytes)))
            .on_attached(Arc::new(move || {
                let _ = events.send(attached.clone());
            }));
        slot.relay.add(lane, Sink::Outbox(synced.outbox()));
        let synced = Arc::new(tokio::sync::Mutex::new(synced));
        lanes.insert(lane.to_owned(), synced.clone());
        drop(lanes);
        synced
    }

    /// One connected session with the hub, until the connection drops
    /// (or the returned future is dropped). `Err` for a transport
    /// failure or a hub session already running; run it again on
    /// reconnect.
    pub async fn run(&self, lane: &DocsLane) -> Result<(), DocsError> {
        self.run_lane(HUB, LaneKind::Hub, lane).await
    }

    /// One connected session with another device of the owner, serving
    /// its [`Self::host`], under the lane key `key` (one session per
    /// key; its endpoint id, say). Its rows merge into this index (a
    /// tombstone wins, the later rename wins), and this device's docs
    /// are registered with it; both sync, and each relays to the other
    /// lanes.
    pub async fn run_peer(&self, key: &str, lane: &DocsLane) -> Result<(), DocsError> {
        if key == HUB || key == HOST_LANE {
            return Err(DocsError::Invalid(format!(
                "`{key}` is a reserved lane key"
            )));
        }
        self.run_lane(key, LaneKind::Peer, lane).await
    }

    async fn run_lane(&self, key: &str, kind: LaneKind, lane: &DocsLane) -> Result<(), DocsError> {
        if !architect::lock(&self.inner.running).insert(key.to_owned()) {
            return Err(DocsError::Internal(format!(
                "a `{key}` session is already running"
            )));
        }
        // Released however the session ends, the future dropped included
        // (a device going offline drops it).
        let _running = RunningLane {
            inner: &self.inner,
            key,
        };
        let mut kicks = self.inner.kicks.subscribe();
        let mut session = Session {
            driver: self,
            key,
            kind,
            lane,
            syncing: tokio::task::JoinSet::new(),
            running: HashMap::new(),
            known: HashSet::new(),
        };
        let res = session.drive(&mut kicks).await;
        session.syncing.abort_all();
        res
    }
}

/// A lane's claim on its key, given back on drop.
struct RunningLane<'a> {
    inner: &'a Inner,
    key: &'a str,
}

impl Drop for RunningLane<'_> {
    fn drop(&mut self) {
        architect::lock(&self.inner.running).remove(self.key);
    }
}

/// One lane's session.
struct Session<'a> {
    driver: &'a DocsDriver,
    key: &'a str,
    kind: LaneKind,
    lane: &'a DocsLane,
    /// Each live doc's sync loop on this lane; dropped (aborted) with
    /// the session.
    syncing: tokio::task::JoinSet<()>,
    running: HashMap<Uuid, tokio::task::AbortHandle>,
    /// A peer lane's docs the peer holds (registered with it, or read
    /// from its catalog): the ones that sync there.
    known: HashSet<Uuid>,
}

impl Session<'_> {
    fn index(&self) -> &dyn DocsIndex {
        &*self.driver.inner.index
    }

    async fn drive(
        &mut self,
        kicks: &mut tokio::sync::broadcast::Receiver<Kick>,
    ) -> Result<(), DocsError> {
        // Subscribe first, so nothing published from here on is missed.
        let (tx, mut rx) = vox::channel::<DocChange>();
        let lane = self.lane;
        let mut subscription = std::pin::pin!(lane.changes.changes(tx));
        let mut attached = false;
        while !attached {
            tokio::select! {
                res = &mut subscription => return res.map_err(transport),
                frame = rx.recv() => match frame {
                    Ok(Some(frame)) => {
                        let mut owned = None;
                        let _ = frame.map(|f| owned = Some(f));
                        attached = matches!(owned, Some(DocChange::Attached { .. }));
                    }
                    _ => return Ok(()),
                },
            }
        }
        self.register().await?;
        self.catch_up().await?;
        self.start_all().await?;

        loop {
            tokio::select! {
                res = &mut subscription => return res.map_err(transport),
                frame = rx.recv() => {
                    let mut owned = None;
                    match frame {
                        Ok(Some(frame)) => {
                            let _ = frame.map(|f| owned = Some(f));
                        }
                        _ => return Ok(()),
                    }
                    if let Some(change) = owned {
                        self.on_change(change).await?;
                    }
                }
                kick = kicks.recv() => {
                    match kick {
                        Ok(Kick::Reset(doc_id)) => {
                            if let Some(task) = self.running.remove(&doc_id) {
                                task.abort();
                            }
                        }
                        Ok(Kick::Changed) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                    self.register().await?;
                    self.start_all().await?;
                }
            }
        }
    }

    /// Register what the host should hold, parent first: on the hub,
    /// every pending doc; on a peer, every live doc it does not hold yet.
    async fn register(&mut self) -> Result<(), DocsError> {
        let docs: Vec<LocalDoc> = self
            .index()
            .docs()
            .await?
            .into_iter()
            .filter(|d| !d.meta.deleted)
            .filter(|d| match self.kind {
                LaneKind::Hub => d.pending,
                LaneKind::Peer => !self.known.contains(&d.meta.doc_id),
            })
            .collect();
        for local in parent_first(docs, |d| (d.meta.doc_id, d.meta.parent)) {
            let m = &local.meta;
            let req = CreateDoc {
                doc_id: m.doc_id,
                kind: m.kind.clone(),
                name: m.name.clone(),
                parent: m.parent,
                lineage: m.lineage,
            };
            match self
                .lane
                .docs
                .create(req)
                .await
                .map_err(architect::vox_into_app)
            {
                Ok(meta) => match self.kind {
                    LaneKind::Hub => {
                        let change = DocChange::Renamed(meta.clone());
                        self.driver
                            .put(
                                LocalDoc {
                                    meta,
                                    pending: false,
                                    seq: 0,
                                },
                                change,
                            )
                            .await?;
                    }
                    LaneKind::Peer => {
                        self.known.insert(m.doc_id);
                    }
                },
                Err(DocsError::Deleted) => {
                    let gone = DocMeta {
                        deleted: true,
                        updated_us: now_us(),
                        ..local.meta.clone()
                    };
                    let change = DocChange::Deleted(gone.clone());
                    self.driver
                        .put(
                            LocalDoc {
                                meta: gone,
                                pending: false,
                                seq: 0,
                            },
                            change,
                        )
                        .await?;
                }
                Err(e @ DocsError::Transport(_)) => return Err(e),
                // A kind or lineage clash, or a refusal: on the hub it
                // stays pending (and unsynced) for the app to resolve.
                Err(e) => {
                    tracing::warn!(doc.id = %m.doc_id, doc.kind = %m.kind, lane = self.key, error = %e, "docs: registering a doc failed");
                }
            }
        }
        Ok(())
    }

    /// Read the lane's catalog from the device's cursor and apply it.
    async fn catch_up(&mut self) -> Result<(), DocsError> {
        let since_rev = self.index().cursor(self.key).await?;
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
        self.index().set_cursor(self.key, page.cursor).await
    }

    /// Take one of the lane's rows into the device's catalog.
    async fn apply(&mut self, meta: DocMeta) -> Result<(), DocsError> {
        let held = self.index().get(meta.doc_id).await?;
        let next = match self.kind {
            LaneKind::Hub => {
                if held.as_ref().is_some_and(|h| h.meta == meta && !h.pending) {
                    return Ok(());
                }
                LocalDoc {
                    meta,
                    pending: false,
                    seq: 0,
                }
            }
            LaneKind::Peer => {
                self.known.insert(meta.doc_id);
                match &held {
                    // Adopted from a peer: the hub has yet to hear of it
                    // from this device (registering is idempotent).
                    None => LocalDoc {
                        meta: DocMeta { rev: 0, ..meta },
                        pending: true,
                        seq: 0,
                    },
                    Some(h) => match merge_peer(&h.meta, &meta) {
                        Some(merged) => LocalDoc {
                            meta: merged,
                            pending: h.pending,
                            seq: 0,
                        },
                        None => return Ok(()),
                    },
                }
            }
        };
        if next.meta.deleted {
            if let Some(task) = self.running.remove(&next.meta.doc_id) {
                task.abort();
            }
        }
        let adopted = held.is_none();
        let change = change_for(held.as_ref(), next.meta.clone());
        self.driver.put(next, change).await?;
        if adopted && self.kind == LaneKind::Peer {
            // The hub session registers it.
            self.driver.kick_changed();
        }
        Ok(())
    }

    /// A frame of the lane's `changes`.
    async fn on_change(&mut self, change: DocChange) -> Result<(), DocsError> {
        let Some(meta) = change.meta().cloned() else {
            return Ok(());
        };
        let cursor = self.index().cursor(self.key).await?;
        if meta.rev <= cursor {
            return Ok(());
        }
        let next = cursor.saturating_add(1);
        if meta.rev > next {
            // The stream dropped something: read the catalog instead.
            self.catch_up().await?;
        } else {
            self.apply(meta).await?;
            self.index().set_cursor(self.key, next).await?;
        }
        self.start_all().await
    }

    /// Start syncing every doc this lane serves that is not syncing yet:
    /// on the hub the registered live docs, on a peer the live docs it
    /// holds.
    async fn start_all(&mut self) -> Result<(), DocsError> {
        self.running.retain(|_, task| !task.is_finished());
        let docs = self.index().docs().await?;
        let ready: Vec<LocalDoc> = docs
            .into_iter()
            .filter(|d| !d.meta.deleted && !self.running.contains_key(&d.meta.doc_id))
            .filter(|d| match self.kind {
                LaneKind::Hub => !d.pending,
                LaneKind::Peer => self.known.contains(&d.meta.doc_id),
            })
            .collect();
        for local in parent_first(ready, |d| (d.meta.doc_id, d.meta.parent)) {
            let doc_id = local.meta.doc_id;
            let slot = match self.driver.slot(&local.meta).await {
                Ok(slot) => slot,
                Err(e) => {
                    tracing::warn!(doc.id = %doc_id, error = %e, "docs: no local storage for a doc");
                    continue;
                }
            };
            let synced = self.driver.lane_replica(&slot, doc_id, self.key).await;
            let client = self.lane.sync.clone();
            let events = self.driver.inner.sync_events.clone();
            let lane = self.key.to_owned();
            let task = self
                .syncing
                .spawn(keep_synced(doc_id, lane, synced, client, events));
            self.running.insert(doc_id, task);
        }
        Ok(())
    }
}

/// Which refusal an attach error is, if it is one.
fn refusal_of(why: &str) -> Option<Refusal> {
    if why.contains(crate::peer::FOREIGN_LINEAGE) {
        Some(Refusal::ForeignLineage)
    } else if why.contains("unknown doc") {
        Some(Refusal::NotServed)
    } else {
        None
    }
}

/// One doc's sync sessions on one lane, retried with backoff while the
/// lane's session lasts. A replica the host refuses as another lineage,
/// or no longer serves, stops here.
// The replica's lock is held for the task's life: one session per doc
// and lane.
#[allow(clippy::significant_drop_tightening)]
async fn keep_synced(
    doc_id: Uuid,
    lane: String,
    synced: Arc<tokio::sync::Mutex<SyncedDoc>>,
    client: DocSyncClient,
    events: tokio::sync::broadcast::Sender<DocSyncEvent>,
) {
    let mut synced = synced.lock().await;
    let mut wait = RETRY_FLOOR;
    loop {
        match synced.run(&client).await {
            Ok(()) => wait = RETRY_FLOOR,
            Err(e) => {
                let why = e.to_string();
                if let Some(reason) = refusal_of(&why) {
                    tracing::warn!(doc.id = %doc_id, lane, error = %why, "docs: the host will not sync this doc");
                    let _ = events.send(DocSyncEvent::Refused {
                        doc_id,
                        lane,
                        reason,
                    });
                    return;
                }
                tracing::debug!(doc.id = %doc_id, lane, error = %why, "docs: doc session ended; retrying");
            }
        }
        let _ = events.send(DocSyncEvent::Detached {
            doc_id,
            lane: lane.clone(),
        });
        tokio::time::sleep(wait).await;
        wait = wait.saturating_mul(2).min(RETRY_CAP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::Lineage;

    fn meta(name: &str, updated_us: i64) -> DocMeta {
        DocMeta {
            doc_id: Uuid::nil(),
            kind: "rig".into(),
            name: name.into(),
            parent: None,
            created_by: String::new(),
            origin: None,
            created_us: 0,
            updated_us,
            rev: 7,
            deleted: false,
            lineage: Lineage::Merge,
        }
    }

    #[test]
    fn a_peers_row_merges_tombstone_first_then_the_later_name() {
        let ours = meta("Bass", 10);
        assert_eq!(merge_peer(&ours, &meta("Bass", 10)), None, "nothing new");
        let renamed = merge_peer(&ours, &meta("Bass rig", 20)).unwrap();
        assert_eq!(
            (renamed.name.as_str(), renamed.rev),
            ("Bass rig", 7),
            "our rev stays"
        );
        assert_eq!(
            merge_peer(&ours, &meta("Old", 5)),
            None,
            "an older name loses"
        );
        let gone = merge_peer(
            &ours,
            &DocMeta {
                deleted: true,
                ..meta("Bass", 1)
            },
        )
        .unwrap();
        assert!(gone.deleted, "a tombstone wins whenever it was made");
    }
}
