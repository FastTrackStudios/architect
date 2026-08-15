//! [`DocRegistry`] — one mounted service pair, **many** collaborative
//! docs.
//!
//! [`DocSyncHost`] and [`PresenceHost`] each serve exactly one `doc_id`;
//! a product like Task wants *one doc per project*, with projects coming
//! and going at runtime. The registry implements the same [`DocSync`] +
//! [`DocPresence`] vox service traits, so a single dispatcher pair on the
//! router serves every doc: on `sync(doc_id, …)` / `presence(doc_id, …)`
//! it looks up — or lazily opens, via the caller-supplied **factory** —
//! the per-doc host and delegates to it. Everything the single-doc hosts
//! already get right (snapshot-then-changes attach, durable relay of
//! updates, weak-handle subscription closures, unbounded doc fan-out)
//! is inherited by composition, not re-implemented.
//!
//! ```ignore
//! let registry = DocRegistry::new(move |doc_id| {
//!     let root = data_dir.clone();
//!     // FilePersistence namespaces by doc_id under `root` on its own;
//!     // a SeaORM backend would key its tables by doc_id the same way.
//!     Box::pin(async move { CrdtDoc::open(doc_id, FilePersistence::new(root)).await })
//! })
//! .with_compaction(64)
//! .with_presence_timeout(30_000)
//! .with_admission(|doc_id| known_projects.contains(&doc_id))
//! .with_idle_eviction(Duration::from_secs(900));
//!
//! router
//!     .with(doc_sync_service_descriptor(), DocSyncDispatcher::new(registry.clone()))
//!     .with(doc_presence_service_descriptor(), DocPresenceDispatcher::new(registry));
//! ```
//!
//! ## Locking
//!
//! One `tokio::sync::Mutex` guards the doc map. It is held across the
//! factory call (so two concurrent first-attaches can't double-open the
//! same persistence) **and** across the delegated attach itself — which
//! is also what makes eviction sound (below). Attaches are quick
//! (export + spawn, no network awaits), so the serialization is cheap;
//! steady-state traffic never touches the registry lock at all — update
//! pumping runs entirely inside the per-doc hosts.
//!
//! ## Eviction vs. in-flight attach
//!
//! The race to guard: the sweeper decides a doc is idle, and a new
//! attach for the same `doc_id` lands between that decision and the
//! removal — the attach would bind to a host the registry just forgot,
//! and the *next* attach would open a second host over the same
//! persistence: two hubs, replicas silently split. The guard is the
//! creation lock: the sweeper re-checks `active_sessions() == 0` and
//! the idle clock **while holding the same mutex** every attach holds
//! from lookup through completion, so an attach either finishes first
//! (sessions > 0 → no evict) or starts after the entry is gone (and
//! re-opens it from persistence).
//!
//! Session counts come from the hosts themselves (incremented on
//! attach, decremented when a session's up-channel closes), not from
//! `PubSub::subscriber_count` — the hub only sweeps dead sinks on
//! publish, so it overcounts on a quiet doc.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::sync::{DocPresence, DocSync, DocSyncHost, PresenceHost, SyncError};
use crate::{CrdtDoc, PersistError};

/// What a [`DocRegistry`] factory returns: a boxed future resolving to
/// the opened doc. Boxed so the factory can be a plain closure —
/// `|doc_id| Box::pin(async move { CrdtDoc::open(doc_id, …).await })`.
pub type DocFuture = Pin<Box<dyn Future<Output = Result<CrdtDoc, PersistError>> + Send>>;

type Factory = dyn Fn(Uuid) -> DocFuture + Send + Sync;
type AdmissionHook = dyn Fn(Uuid) -> bool + Send + Sync;

/// One open doc: its sync host, its presence host, and when the
/// registry last touched it (attach or [`DocRegistry::doc`]).
struct DocEntry {
    sync: DocSyncHost,
    presence: PresenceHost,
    last_used: Instant,
}

/// State shared by every clone of the registry (and weakly by the
/// eviction sweeper, so the sweeper never keeps a dropped registry
/// alive).
struct Shared {
    factory: Box<Factory>,
    docs: tokio::sync::Mutex<HashMap<Uuid, DocEntry>>,
}

/// A multi-doc [`DocSync`] + [`DocPresence`] server: per-doc
/// [`DocSyncHost`]s / [`PresenceHost`]s, opened on demand through the
/// factory and addressed by the `doc_id` already present in both wire
/// calls. Cheap to clone — all clones share the doc map.
///
/// Configure (`with_*`) **before** mounting/cloning: the builder
/// methods set plain fields that are copied into each clone, and the
/// settings are applied when a doc's hosts are created.
#[derive(Clone)]
pub struct DocRegistry {
    shared: Arc<Shared>,
    admission: Arc<AdmissionHook>,
    compact_every: u32,
    shallow_bootstrap: bool,
    presence_timeout_ms: i64,
}

impl DocRegistry {
    /// A registry whose docs are opened by `factory`. The factory owns
    /// the per-doc persistence policy: [`FilePersistence`](crate::fs::FilePersistence)
    /// already namespaces by doc id under its root; a database backend
    /// would key its rows by `doc_id` the same way. Default settings:
    /// no compaction, full-history bootstrap, 30 s presence timeout,
    /// allow-all admission, no eviction.
    pub fn new<F>(factory: F) -> Self
    where
        F: Fn(Uuid) -> DocFuture + Send + Sync + 'static,
    {
        Self {
            shared: Arc::new(Shared {
                factory: Box::new(factory),
                docs: tokio::sync::Mutex::new(HashMap::new()),
            }),
            admission: Arc::new(|_| true),
            compact_every: 0,
            shallow_bootstrap: false,
            presence_timeout_ms: 30_000,
        }
    }

    /// Forwarded to every created host: compact each doc's persistence
    /// every `n` updates ([`DocSyncHost::with_compaction`]).
    pub fn with_compaction(mut self, n: u32) -> Self {
        self.compact_every = n;
        self
    }

    /// Forwarded to every created host: bootstrap fresh joiners with a
    /// shallow snapshot ([`DocSyncHost::with_shallow_bootstrap`]).
    pub fn with_shallow_bootstrap(mut self) -> Self {
        self.shallow_bootstrap = true;
        self
    }

    /// Presence expiry for every created [`PresenceHost`] — how long a
    /// peer's state survives without an update (default 30 000 ms).
    pub fn with_presence_timeout(mut self, timeout_ms: i64) -> Self {
        self.presence_timeout_ms = timeout_ms;
        self
    }

    /// Gate which doc ids this registry will open or serve at all.
    /// Rejected ids fail with [`SyncError::UnknownDoc`] — indistinguishable
    /// from a doc that doesn't exist, so the hook doesn't leak which ids
    /// are real. Default: allow all.
    ///
    /// This is **coarse server-side scoping** (e.g. "only docs this
    /// deployment owns"), not authorization: the hook sees only the doc
    /// id, not who is asking. Real per-user authz needs caller identity
    /// threaded through the transport (future work — vox middleware
    /// context).
    pub fn with_admission<H>(mut self, hook: H) -> Self
    where
        H: Fn(Uuid) -> bool + Send + Sync + 'static,
    {
        self.admission = Arc::new(hook);
        self
    }

    /// Evict docs that have had **zero live sessions** (sync and
    /// presence) and no registry activity for `after`: a sweeper task
    /// compacts the doc's persistence (so the next open replays one
    /// snapshot, not a long update log) and drops its hosts. The next
    /// attach re-opens the doc through the factory — by construction it
    /// sees everything the evicted host persisted, because relayed
    /// updates are written durably *before* they're fanned out.
    ///
    /// The sweeper holds the registry only weakly and exits when the
    /// registry is dropped. Must be called inside a tokio runtime (the
    /// sweeper is a tokio task on a timer).
    ///
    /// See the module docs for the eviction-vs-in-flight-attach race
    /// and why re-checking under the creation lock closes it. One
    /// caveat outside the lock's reach: a [`CrdtDoc`] handle obtained
    /// from [`Self::doc`] and held across an eviction keeps the *old*
    /// doc alive while new attaches get a fresh one — writes through a
    /// stale handle reach persistence but not live replicas. Don't
    /// enable eviction if server-side code holds long-lived doc
    /// handles (or re-fetch the handle per use).
    pub fn with_idle_eviction(self, after: Duration) -> Self {
        let weak = Arc::downgrade(&self.shared);
        let period = (after / 2).max(Duration::from_millis(25));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(period).await;
                let Some(shared) = weak.upgrade() else { break };
                // The creation lock — held while deciding *and* removing,
                // the same lock every attach holds through completion.
                let mut docs = shared.docs.lock().await;
                let idle: Vec<Uuid> = docs
                    .iter()
                    .filter(|(_, e)| {
                        e.sync.active_sessions() == 0
                            && e.presence.active_sessions() == 0
                            && e.last_used.elapsed() >= after
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for doc_id in idle {
                    let Some(entry) = docs.remove(&doc_id) else {
                        continue;
                    };
                    // Compact before dropping so reopen is one snapshot
                    // import. On failure evict anyway — the update log
                    // is still intact, reopen just replays it.
                    if let Err(e) = entry.sync.doc().compact(doc_id).await {
                        tracing::warn!(?doc_id, "doc-registry: pre-evict compaction failed: {e}");
                    }
                    tracing::debug!(?doc_id, "doc-registry: evicted idle doc");
                }
            }
        });
        self
    }

    /// The canonical doc for `doc_id`, opening it (through the factory)
    /// if needed — for server-side reads/writes via entity repos.
    /// Honors the admission hook. Counts as activity for eviction; see
    /// [`Self::with_idle_eviction`] for the stale-handle caveat.
    pub async fn doc(&self, doc_id: Uuid) -> Result<CrdtDoc, SyncError> {
        if !(self.admission)(doc_id) {
            return Err(SyncError::UnknownDoc);
        }
        let mut docs = self.shared.docs.lock().await;
        let entry = self.entry(&mut docs, doc_id).await?;
        Ok(entry.sync.doc().clone())
    }

    /// Whether `doc_id` currently has open hosts (it may have been
    /// evicted, or never attached). Observability + tests.
    pub async fn is_open(&self, doc_id: Uuid) -> bool {
        self.shared.docs.lock().await.contains_key(&doc_id)
    }

    /// Ids of every currently open doc.
    pub async fn open_docs(&self) -> Vec<Uuid> {
        self.shared.docs.lock().await.keys().copied().collect()
    }

    /// Look up — or open and host — `doc_id`. Caller holds the map
    /// lock, which is what makes "check, open, insert" atomic against
    /// a concurrent first-attach for the same id.
    async fn entry<'a>(
        &self,
        docs: &'a mut HashMap<Uuid, DocEntry>,
        doc_id: Uuid,
    ) -> Result<&'a mut DocEntry, SyncError> {
        if let std::collections::hash_map::Entry::Vacant(slot) = docs.entry(doc_id) {
            let doc = (self.shared.factory)(doc_id)
                .await
                .map_err(|e| SyncError::Internal(format!("open doc {doc_id}: {e}")))?;
            let mut host = DocSyncHost::new(doc_id, doc).with_compaction(self.compact_every);
            if self.shallow_bootstrap {
                host = host.with_shallow_bootstrap();
            }
            slot.insert(DocEntry {
                sync: host,
                presence: PresenceHost::new(doc_id, self.presence_timeout_ms),
                last_used: Instant::now(),
            });
        }
        let entry = docs.get_mut(&doc_id).expect("entry just ensured");
        entry.last_used = Instant::now();
        Ok(entry)
    }
}

impl DocSync for DocRegistry {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<crate::sync::SyncDown>,
    ) -> Result<(), SyncError> {
        if !(self.admission)(doc_id) {
            return Err(SyncError::UnknownDoc);
        }
        // Attach while holding the registry lock — the session is
        // counted before any sweep can re-check — but pump the held-open
        // call AFTER releasing it: `sync` now stays in flight for the
        // whole session (vox 0.10 channel scoping), and a lock held
        // across it would wedge every other doc's attach.
        let session = {
            let mut docs = self.shared.docs.lock().await;
            let entry = self.entry(&mut docs, doc_id).await?;
            entry.sync.attach(doc_id, from, down)?
        };
        session.pump(up).await;
        Ok(())
    }
}

impl DocPresence for DocRegistry {
    async fn presence(
        &self,
        doc_id: Uuid,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), SyncError> {
        if !(self.admission)(doc_id) {
            return Err(SyncError::UnknownDoc);
        }
        // Same attach-under-lock / pump-after-release split as `sync`.
        let session = {
            let mut docs = self.shared.docs.lock().await;
            let entry = self.entry(&mut docs, doc_id).await?;
            entry.presence.attach(doc_id, down)?
        };
        session.pump(up).await;
        Ok(())
    }
}
