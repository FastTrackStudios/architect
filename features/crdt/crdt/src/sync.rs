//! Real-time, offline-first doc sync over vox — the transport layer that
//! turns a [`CrdtDoc`] into a **collaborative replica**.
//!
//! Loro gives us the hard part for free: every replica holds the full
//! document, local writes apply instantly (offline-by-construction), and
//! `import` merges any update from any peer, in any order, idempotently.
//! What's left is moving update bytes around, and that is one vox call:
//!
//! ```text
//!  client A                      server                       client B
//!  ┌────────────┐   sync(vv,up,down)  ┌──────────────┐  sync   ┌────────────┐
//!  │ CrdtDoc    │ ──────────────────▶ │ canonical    │ ◀────── │ CrdtDoc    │
//!  │ (replica)  │  ◀── backlog ────── │ CrdtDoc      │ ──────▶ │ (replica)  │
//!  │ local ops ─┼──── up channel ───▶ │ + Persistence│ ─down─▶ │            │
//!  │            │ ◀─── down channel ──│ + PubSub     │         │            │
//!  └────────────┘                     └──────────────┘         └────────────┘
//! ```
//!
//! One [`DocSync::sync`] call per client: it carries the client's version
//! vector (so the server replies exactly the missing history — delta
//! catch-up after any offline period), an **up** channel of the client's
//! future local updates, and a **down** sink. The server merges every
//! incoming update into the canonical doc (persisting it), and fans it
//! out to every other replica through an [`architect::PubSub`] — the same
//! snapshot-then-changes attach discipline as entity events, so nothing
//! is missed between catch-up and live traffic.
//!
//! There is **no rollback** in this model — concurrent edits merge. The
//! optimistic store's reconcile/rollback machinery is for server-owned
//! state; CRDT-backed features replace it with a local replica that is
//! always written synchronously and always converges.
//!
//! The pieces:
//! - [`DocSync`] — the `#[vox::service]` wire trait (client + dispatcher
//!   emitted as usual).
//! - [`DocSyncHost`] — the server: canonical doc + fan-out. Native only
//!   (it spawns a pump per subscriber).
//! - [`SyncedDoc`] — the client driver: wires a local [`CrdtDoc`]'s
//!   update stream to the sync call and applies what comes back. Returns
//!   its pump as a future so any spawner works (`tokio::spawn`,
//!   `dioxus::spawn`, `wasm_bindgen_futures`) — wasm-clean.

use std::sync::Arc;

use uuid::Uuid;

use crate::{CrdtDoc, PersistError};

/// Why a sync attach failed.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet, thiserror::Error)]
#[repr(u8)]
pub enum SyncError {
    #[error("unknown doc")]
    UnknownDoc,
    #[error("bad version vector: {0}")]
    BadVersion(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// A frame on the sync **down** channel.
///
/// vox 0.10 scopes channels to their request — delivering the response
/// terminates the channels (`RequestTerminated(ResponseDelivered)`), so
/// the attach can't return a value and keep streaming. The server's
/// version vector therefore rides the down channel as the first frame
/// instead of the return value, and the [`DocSync::sync`] call itself
/// stays in flight for the whole session.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum SyncDown {
    /// First frame after a successful attach: the **server's** encoded
    /// version vector (so the client can push back any history the
    /// server is missing — a replica that edited offline, restarted,
    /// then reconnected: its outbox is gone but its doc still holds the
    /// ops; catch-up is bidirectional by construction) plus the exact
    /// history the client is missing (may be empty for an up-to-date
    /// replica).
    Attached {
        server_vv: Vec<u8>,
        backlog: Vec<u8>,
    },
    /// A Loro update from any peer (or the server itself), live.
    Update(Vec<u8>),
}

/// The sync wire trait. `from` is the client's encoded Loro version
/// vector (empty for a fresh replica). On attach the server sends
/// [`SyncDown::Attached`] (server version vector + missing history)
/// through `down`, then every subsequent update from any peer. Updates
/// the client makes flow through `up`.
///
/// **The call is the session** (vox 0.10 channel scoping): a successful
/// attach holds the request open — `sync` only returns once the
/// replica's connection closes (or the client cancels the in-flight
/// call, which is the unsubscribe). An attach *failure* returns the
/// error immediately.
#[vox::service]
pub trait DocSync {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError>;
}

/// Presence wire trait — who's here and what they're doing (cursors,
/// selections, online status). Same session shape as [`DocSync`] (the
/// in-flight call is the session; it only returns when the peer's
/// connection closes), but the payloads are Loro `EphemeralStore`
/// encodings: timestamp-LWW state that expires on its own, never
/// persisted, so no version-vector envelope is needed. On attach the
/// server sends the current presence picture, then live updates.
#[vox::service]
pub trait DocPresence {
    async fn presence(
        &self,
        doc_id: Uuid,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), SyncError>;
}

// ── Server ──────────────────────────────────────────────────────────────

/// The server side of one collaborative doc: the canonical [`CrdtDoc`]
/// (with its persistence) plus the update fan-out.
///
/// Every update — whether it arrived from a replica's up-channel or was
/// made locally on the server (another transport mutating the same doc
/// through its `LoroRepo`s) — is published to every attached replica.
/// Loro imports are idempotent, so echoing a sender its own update is
/// harmless; replicas simply converge.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct DocSyncHost {
    doc_id: Uuid,
    doc: CrdtDoc,
    hub: architect::PubSub<SyncDown>,
    /// Live sync sessions: incremented on a successful attach, decremented
    /// when the session's up-pump ends (the replica's connection closed).
    /// What [`DocRegistry`](crate::registry::DocRegistry) consults before
    /// evicting an idle host — `PubSub::subscriber_count` is only swept
    /// lazily (on publish), so it can overcount on a quiet doc.
    sessions: Arc<std::sync::atomic::AtomicUsize>,
    /// Updates (local or relayed) since the last compaction trigger.
    update_count: Arc<std::sync::atomic::AtomicU32>,
    /// 0 = compaction off. Shared with the local-update bridge so
    /// [`Self::with_compaction`] can configure it after construction.
    compact_every: Arc<std::sync::atomic::AtomicU32>,
    /// Wakes the compaction worker (which holds only a weak doc handle
    /// — see the deadlock note on [`CrdtDoc::downgrade`]).
    compact_tx: tokio::sync::mpsc::UnboundedSender<()>,
    /// Bootstrap fresh joiners (empty version vector) with a shallow
    /// snapshot instead of full history — bounded first-sync for
    /// long-lived docs. Replicas that already hold history still get
    /// exact deltas.
    shallow_bootstrap: bool,
    // Keeps the server-local-update → hub bridge alive.
    _local_sub: Arc<loro::Subscription>,
}

#[cfg(not(target_arch = "wasm32"))]
impl DocSyncHost {
    /// Wrap a canonical doc for serving. Server-side writes to the same
    /// doc (in-process repos, other transports) broadcast automatically.
    pub fn new(doc_id: Uuid, doc: CrdtDoc) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        // Unbounded: update bytes must never be dropped (unlike state-
        // shaped entity events, a missed CRDT update is a lost edit until
        // the next full catch-up).
        let hub = architect::PubSub::unbounded();
        let update_count = Arc::new(AtomicU32::new(0));
        let compact_every = Arc::new(AtomicU32::new(0));
        let (compact_tx, mut compact_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        // The compaction worker. Holds the doc WEAKLY: a strong handle
        // here (or in the subscription closure below) could become the
        // doc's last owner and make it drop inside loro's unsubscribe
        // path — a deadlock (see `CrdtDoc::downgrade`). The worker ends
        // when every sender is gone.
        let weak = doc.downgrade();
        tokio::spawn(async move {
            while compact_rx.recv().await.is_some() {
                let Some(doc) = weak.upgrade() else { break };
                if let Err(e) = doc.compact(doc_id).await {
                    tracing::warn!(?doc_id, "doc-sync: compaction failed: {e}");
                }
            }
        });

        let bridge = hub.clone();
        let count = update_count.clone();
        let every = compact_every.clone();
        let trigger = compact_tx.clone();
        let sub = doc.loro().subscribe_local_update(Box::new(move |bytes| {
            bridge.publish(SyncDown::Update(bytes.to_vec()));
            if should_compact(&count, every.load(Ordering::Relaxed)) {
                let _ = trigger.send(());
            }
            true
        }));
        Self {
            doc_id,
            doc,
            hub,
            sessions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            update_count,
            compact_every,
            compact_tx,
            shallow_bootstrap: false,
            _local_sub: Arc::new(sub),
        }
    }

    /// Compact the canonical doc's persistence every `n` updates: fold
    /// the accumulated update log into one snapshot (via
    /// [`CrdtDoc::compact`]) so server storage stays bounded no matter
    /// how chatty the doc is. Counts both server-local commits and
    /// updates relayed from replicas.
    pub fn with_compaction(self, n: u32) -> Self {
        self.compact_every
            .store(n, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// Serve fresh joiners (empty version vector) a shallow snapshot —
    /// current state with truncated history — instead of the full op
    /// log. Right for long-lived docs where a new device doesn't need
    /// to merge against months of edits. Reconnecting replicas (non-
    /// empty version vector) are unaffected and still get exact deltas.
    pub fn with_shallow_bootstrap(mut self) -> Self {
        self.shallow_bootstrap = true;
        self
    }

    /// The canonical doc (for mounting entity repos on the server).
    pub fn doc(&self) -> &CrdtDoc {
        &self.doc
    }

    /// How many sync sessions are currently attached. A session counts
    /// from a successful [`DocSync::sync`] until its up-channel closes
    /// (i.e. the replica's connection tore down).
    pub fn active_sessions(&self) -> usize {
        self.sessions.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// RAII session counter: increments on construction, decrements on drop
/// (the host pump task owns one for its lifetime).
#[cfg(not(target_arch = "wasm32"))]
struct SessionGuard(Arc<std::sync::atomic::AtomicUsize>);

#[cfg(not(target_arch = "wasm32"))]
impl SessionGuard {
    fn new(counter: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self(counter)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Bump the shared update counter; `true` when it crosses the threshold
/// (and resets). Threshold 0 disables compaction.
#[cfg(not(target_arch = "wasm32"))]
fn should_compact(count: &std::sync::atomic::AtomicU32, every: u32) -> bool {
    use std::sync::atomic::Ordering;
    if every == 0 {
        return false;
    }
    if count.fetch_add(1, Ordering::Relaxed) + 1 < every {
        return false;
    }
    count.store(0, Ordering::Relaxed);
    true
}

/// One attached sync session, produced by [`DocSyncHost::attach`] and
/// driven to completion by [`SyncSession::pump`]. Owns the session's
/// `SessionGuard`, so the session is counted from the attach (inside
/// whatever lock the caller holds — see
/// [`DocRegistry`](crate::registry::DocRegistry)) until the pump ends.
#[cfg(not(target_arch = "wasm32"))]
pub struct SyncSession {
    doc: CrdtDoc,
    doc_id: Uuid,
    hub: architect::PubSub<SyncDown>,
    count: Arc<std::sync::atomic::AtomicU32>,
    every: Arc<std::sync::atomic::AtomicU32>,
    trigger: tokio::sync::mpsc::UnboundedSender<()>,
    _session: SessionGuard,
}

#[cfg(not(target_arch = "wasm32"))]
impl SyncSession {
    /// Pump this replica's local updates into the canonical doc —
    /// durably (imports don't fire the persistence subscription) — and
    /// out to everyone else. Runs until the replica's connection closes
    /// (the up channel ends). This is the body of the held-open
    /// [`DocSync::sync`] call — **do not** hold any registry/host lock
    /// across it.
    pub async fn pump(self, mut up: vox::Rx<Vec<u8>>) {
        while let Ok(Some(update)) = up.recv().await {
            let mut owned: Option<Vec<u8>> = None;
            let _ = update.map(|u| owned = Some(u.clone()));
            let Some(bytes) = owned else { continue };
            if let Err(e) = self.doc.apply_remote_durable(self.doc_id, &bytes).await {
                tracing::warn!("doc-sync: dropping bad update: {e}");
                continue;
            }
            self.hub.publish(SyncDown::Update(bytes));
            if should_compact(
                &self.count,
                self.every.load(std::sync::atomic::Ordering::Relaxed),
            ) {
                let _ = self.trigger.send(());
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl DocSyncHost {
    /// The attach phase of [`DocSync::sync`], separated from the pump so
    /// a multi-doc server ([`DocRegistry`](crate::registry::DocRegistry))
    /// can attach under its lock (session counted before any sweep can
    /// re-check) and pump **after** releasing it — a held-open call must
    /// never keep a lock.
    pub fn attach(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        down: vox::Tx<SyncDown>,
    ) -> Result<SyncSession, SyncError> {
        if doc_id != self.doc_id {
            return Err(SyncError::UnknownDoc);
        }
        let fresh_join = from.is_empty();
        let from = if fresh_join {
            loro::VersionVector::new()
        } else {
            loro::VersionVector::decode(&from).map_err(|e| SyncError::BadVersion(e.to_string()))?
        };

        // Snapshot-then-changes: park the subscriber, export exactly the
        // history the replica is missing, deliver it ahead of anything
        // published since the park. (Overlap is fine — imports merge.)
        // Fresh joiners optionally get a shallow snapshot — bounded
        // first-sync regardless of doc age.
        let pending = self.hub.begin_attach(down);
        let backlog = if fresh_join && self.shallow_bootstrap {
            self.doc
                .loro()
                .export(loro::ExportMode::ShallowSnapshot(std::borrow::Cow::Owned(
                    self.doc.loro().oplog_frontiers(),
                )))
        } else {
            self.doc.loro().export(loro::ExportMode::Updates {
                from: std::borrow::Cow::Owned(from),
            })
        };
        let backlog = match backlog {
            Ok(b) => b,
            Err(e) => {
                self.hub.abort_attach(pending);
                return Err(SyncError::Internal(format!("export updates: {e}")));
            }
        };
        // The server's frontier *at attach* — the client diffs its own
        // doc against this and pushes back whatever the server lacks.
        let server_vv = self.doc.loro().oplog_vv().encode();
        self.hub
            .complete_attach(pending, Some(SyncDown::Attached { server_vv, backlog }));

        Ok(SyncSession {
            doc: self.doc.clone(),
            doc_id: self.doc_id,
            hub: self.hub.clone(),
            count: self.update_count.clone(),
            every: self.compact_every.clone(),
            trigger: self.compact_tx.clone(),
            _session: SessionGuard::new(self.sessions.clone()),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl DocSync for DocSyncHost {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError> {
        // The call is the session (vox 0.10 channel scoping): attach,
        // then hold the request open pumping until the replica's
        // connection closes.
        let session = self.attach(doc_id, from, down)?;
        session.pump(up).await;
        Ok(())
    }
}

// ── Presence host ───────────────────────────────────────────────────────

/// The server side of one doc's presence channel: a mirror
/// [`EphemeralStore`](crate::awareness::EphemeralStore) (so late
/// joiners get the current picture on attach) + sliding fan-out
/// (presence is state-shaped — under pressure, dropping a stale cursor
/// position in favor of a newer one is correct).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct PresenceHost {
    doc_id: Uuid,
    store: crate::awareness::EphemeralStore,
    hub: architect::PubSub<Vec<u8>>,
    /// Live presence sessions — same lifecycle as
    /// [`DocSyncHost::active_sessions`].
    sessions: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(not(target_arch = "wasm32"))]
impl PresenceHost {
    /// `timeout_ms` — how long a peer's state survives without an
    /// update before it's considered gone (Loro's ephemeral timeout).
    pub fn new(doc_id: Uuid, timeout_ms: i64) -> Self {
        Self {
            doc_id,
            store: crate::awareness::EphemeralStore::new(timeout_ms),
            hub: architect::PubSub::sliding(64),
            sessions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// The mirror store (e.g. for server-side "who's online" reads).
    pub fn store(&self) -> &crate::awareness::EphemeralStore {
        &self.store
    }

    /// How many presence sessions are currently attached — counted from
    /// a successful [`DocPresence::presence`] until the peer's up-channel
    /// closes.
    pub fn active_sessions(&self) -> usize {
        self.sessions.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// One attached presence session — the counterpart of [`SyncSession`]
/// for [`PresenceHost`]. Same split, same reason (attach under a lock,
/// pump the held-open call after releasing it).
#[cfg(not(target_arch = "wasm32"))]
pub struct PresenceSession {
    store: crate::awareness::EphemeralStore,
    hub: architect::PubSub<Vec<u8>>,
    _session: SessionGuard,
}

#[cfg(not(target_arch = "wasm32"))]
impl PresenceSession {
    /// Merge this peer's updates into the mirror store and fan them out.
    /// Runs until the peer's connection closes. The body of the
    /// held-open [`DocPresence::presence`] call — **do not** hold any
    /// registry/host lock across it.
    pub async fn pump(self, mut up: vox::Rx<Vec<u8>>) {
        while let Ok(Some(update)) = up.recv().await {
            let mut owned: Option<Vec<u8>> = None;
            let _ = update.map(|u| owned = Some(u.clone()));
            let Some(bytes) = owned else { continue };
            if let Err(e) = self.store.apply(&bytes) {
                tracing::warn!("doc-presence: dropping bad update: {e}");
                continue;
            }
            self.hub.publish(bytes);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PresenceHost {
    /// The attach phase of [`DocPresence::presence`] — see
    /// [`DocSyncHost::attach`] for the split's rationale.
    pub fn attach(
        &self,
        doc_id: Uuid,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<PresenceSession, SyncError> {
        if doc_id != self.doc_id {
            return Err(SyncError::UnknownDoc);
        }
        // Same attach discipline as doc sync: park, snapshot the current
        // presence picture, deliver it ahead of live traffic.
        let pending = self.hub.begin_attach(down);
        self.store.remove_outdated();
        let snapshot = self.store.encode_all();
        let snapshot = (!snapshot.is_empty()).then_some(snapshot);
        self.hub.complete_attach(pending, snapshot);

        Ok(PresenceSession {
            store: self.store.clone(),
            hub: self.hub.clone(),
            _session: SessionGuard::new(self.sessions.clone()),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl DocPresence for PresenceHost {
    async fn presence(
        &self,
        doc_id: Uuid,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), SyncError> {
        // Held open like `DocSync::sync` — the in-flight call is the
        // presence session.
        let session = self.attach(doc_id, down)?;
        session.pump(up).await;
        Ok(())
    }
}

// ── Presence client ─────────────────────────────────────────────────────

/// One peer's presence: a cloneable handle for reading/writing the
/// local [`EphemeralStore`](crate::awareness::EphemeralStore), plus a
/// [`PresenceDriver`] that runs the wire session. Convention: each
/// peer writes its own key(s) (e.g. its client id); everyone reads
/// `states()` for the full picture. Values expire on their own after
/// `timeout_ms` without updates.
#[derive(Clone)]
pub struct PresencePeer {
    store: crate::awareness::EphemeralStore,
    /// Keys this peer owns — re-announced on every (re)attach so a
    /// reconnect doesn't leave us invisible until the next change.
    local_keys: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
}

impl PresencePeer {
    /// Build the handle + its driver. Spawn `driver.run(&client)` (and
    /// re-run on disconnect); keep the handle for reads/writes.
    pub fn new(doc_id: Uuid, timeout_ms: i64) -> (Self, PresenceDriver) {
        let store = crate::awareness::EphemeralStore::new(timeout_ms);
        let (tx, outbox) = tokio::sync::mpsc::unbounded_channel();
        let sub = store.subscribe_local_updates(Box::new(move |bytes| {
            let _ = tx.send(bytes.to_vec());
            true
        }));
        let peer = Self {
            store,
            local_keys: Arc::new(std::sync::Mutex::new(Default::default())),
        };
        let driver = PresenceDriver {
            doc_id,
            peer: peer.clone(),
            outbox,
            _local_sub: sub,
        };
        (peer, driver)
    }

    /// Publish this peer's state under `key`. Live-broadcast while
    /// connected, re-announced on reconnect, expired by timeout when
    /// this peer vanishes.
    pub fn set(&self, key: &str, value: impl Into<loro::LoroValue>) {
        self.local_keys.lock().unwrap().insert(key.to_string());
        self.store.set(key, value);
    }

    /// Withdraw a key explicitly (leaving a page, signing out).
    pub fn delete(&self, key: &str) {
        self.local_keys.lock().unwrap().remove(key);
        self.store.delete(key);
    }

    /// Everyone's current state. Expired peers are pruned by the
    /// driver's housekeeping (never here — this is called from render
    /// paths, and `remove_outdated` fires subscriber events, which
    /// would feed the change-subscription back into another render).
    pub fn states(&self) -> std::collections::HashMap<String, loro::LoroValue> {
        self.store.get_all_states().into_iter().collect()
    }

    /// Prune peers whose state outlived the timeout, notifying
    /// subscribers (`Timeout` events). Call this from a task on a
    /// timer — **never from a render path**: pruning fires the store's
    /// change subscription, and a render that prunes re-renders itself
    /// forever (a main-thread livelock in the browser).
    pub fn sweep(&self) {
        self.store.remove_outdated();
    }

    /// The underlying store — for change subscriptions.
    pub fn store(&self) -> &crate::awareness::EphemeralStore {
        &self.store
    }
}

/// The wire session for one [`PresencePeer`]. Same lifecycle as
/// [`SyncedDoc::run`]: one call per connection, re-run on disconnect.
pub struct PresenceDriver {
    doc_id: Uuid,
    peer: PresencePeer,
    outbox: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    _local_sub: loro::Subscription,
}

impl PresenceDriver {
    /// Attach, re-announce our keys, then exchange updates until the
    /// connection drops or the future is cancelled.
    ///
    /// Like [`SyncedDoc::run`], the `presence` call is held in flight
    /// for the whole session; this races it against an up pump and a
    /// down pump in one select, so the call keeps being polled while a
    /// send awaits (a send made before the request has even gone out
    /// would otherwise wedge). The re-announce frames go up first —
    /// they sit in the up channel until the server's pump (which starts
    /// after its attach) reads them, so they can't be lost to the
    /// attach race.
    pub async fn run(&mut self, client: &DocPresenceClient) -> Result<(), PersistError> {
        let (up_tx, up_rx) = vox::channel::<Vec<u8>>();
        let (down_tx, mut down_rx) = vox::channel::<Vec<u8>>();
        let call = std::pin::pin!(client.presence(self.doc_id, up_rx, down_tx));

        // Re-announce what we own — after a reconnect the server's
        // mirror may have expired us.
        let keys: Vec<String> = self
            .peer
            .local_keys
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        let announcements: Vec<Vec<u8>> = keys
            .into_iter()
            .map(|key| self.peer.store.encode(&key))
            .filter(|encoded| !encoded.is_empty())
            .collect();

        let outbox = &mut self.outbox;
        let up = async move {
            for encoded in announcements {
                if up_tx.send(encoded).await.is_err() {
                    return;
                }
            }
            while let Some(update) = outbox.recv().await {
                if up_tx.send(update).await.is_err() {
                    return;
                }
            }
            // Outbox closed (the peer handle was dropped) — the session
            // stays up for reads until the call or down side ends it.
            std::future::pending::<()>().await
        };

        let store = self.peer.store.clone();
        let down = async move {
            loop {
                match down_rx.recv().await {
                    Ok(Some(update)) => {
                        let mut owned: Option<Vec<u8>> = None;
                        let _ = update.map(|u| owned = Some(u.clone()));
                        if let Some(bytes) = owned
                            && let Err(e) = store.apply(&bytes)
                        {
                            tracing::warn!("presence: dropping bad update: {e}");
                        }
                    }
                    _ => return,
                }
            }
        };

        tokio::select! {
            res = call => match res {
                Ok(()) => Ok(()),
                Err(e) => Err(PersistError::Backend(format!("presence attach: {e}"))),
            },
            () = up => Ok(()),   // up channel gone — connection closed
            () = down => Ok(()), // down stream ended
        }
    }
}

// ── Client ──────────────────────────────────────────────────────────────

/// The client driver for one replica: connect a local [`CrdtDoc`] to a
/// [`DocSyncClient`] and the doc becomes collaborative — local writes
/// stream up, remote writes merge in, and a dropped connection just means
/// the next [`SyncedDoc::run`] catches up by version vector.
pub struct SyncedDoc {
    doc: CrdtDoc,
    doc_id: Uuid,
    outbox: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    _local_sub: loro::Subscription,
}

impl SyncedDoc {
    /// Wire a doc for syncing. Do this **once** per replica, before
    /// making local edits, so every local update lands in the outbox.
    pub fn new(doc_id: Uuid, doc: CrdtDoc) -> Self {
        let (tx, outbox) = tokio::sync::mpsc::unbounded_channel();
        let sub = doc.loro().subscribe_local_update(Box::new(move |bytes| {
            // Buffered while offline; drained by `run` when connected.
            let _ = tx.send(bytes.to_vec());
            true
        }));
        Self {
            doc,
            doc_id,
            outbox,
            _local_sub: sub,
        }
    }

    /// The local replica — hand out `doc().repo::<E>()` views to the UI.
    pub fn doc(&self) -> &CrdtDoc {
        &self.doc
    }

    /// Run one sync session to completion: catch up (by version vector),
    /// then exchange updates until the connection drops or the future is
    /// cancelled. Spawn it with whatever executor the platform has; on
    /// disconnect, call again with a fresh client — the version vector
    /// makes re-sync a delta, not a re-download.
    ///
    /// The `sync` call is held in flight for the whole session (vox 0.10
    /// channel scoping — a delivered response terminates the channels),
    /// so this races the in-flight call against the down channel: the
    /// first [`SyncDown::Attached`] frame carries the server's version
    /// vector (for bidirectional catch-up) + our missing history, and
    /// the call future only resolves when the session ends (server gone,
    /// or an attach error).
    pub async fn run(&mut self, client: &DocSyncClient) -> Result<(), PersistError> {
        let (up_tx, up_rx) = vox::channel::<Vec<u8>>();
        let (down_tx, mut down_rx) = vox::channel::<SyncDown>();
        let our_vv = self.doc.loro().oplog_vv();
        let mut call = std::pin::pin!(client.sync(self.doc_id, our_vv.encode(), up_rx, down_tx));

        loop {
            tokio::select! {
                // The held-open call resolved: attach failed, or the
                // server ended the session. Either way this session is
                // over; the caller re-runs with a fresh client.
                res = &mut call => {
                    return match res {
                        Ok(()) => Ok(()),
                        Err(e) => Err(PersistError::Backend(format!("doc sync attach: {e}"))),
                    };
                }
                // Local edits (live, or buffered while offline) go up.
                Some(update) = self.outbox.recv() => {
                    if up_tx.send(update).await.is_err() {
                        return Ok(()); // connection gone; caller re-runs
                    }
                }
                // Attach frame, then remote history + live peer updates.
                incoming = down_rx.recv() => {
                    let mut owned: Option<SyncDown> = None;
                    match incoming {
                        Ok(Some(frame)) => {
                            let _ = frame.map(|f| owned = Some(f.clone()));
                        }
                        _ => return Ok(()), // stream ended
                    }
                    match owned {
                        Some(SyncDown::Attached { server_vv, backlog }) => {
                            if !backlog.is_empty() {
                                self.doc.apply_remote(&backlog)?;
                            }
                            // Bidirectional catch-up: if we hold history
                            // the server lacks (edited offline, restarted
                            // — the in-memory outbox is gone but the doc
                            // isn't), push the exact missing delta.
                            let server_vv = loro::VersionVector::decode(&server_vv).map_err(
                                |e| PersistError::Backend(format!("doc sync: bad server vv: {e}")),
                            )?;
                            if !server_vv.includes_vv(&our_vv) {
                                let missing = self
                                    .doc
                                    .loro()
                                    .export(loro::ExportMode::Updates {
                                        from: std::borrow::Cow::Owned(server_vv),
                                    })
                                    .map_err(|e| {
                                        PersistError::Backend(format!(
                                            "doc sync: export delta: {e}"
                                        ))
                                    })?;
                                if up_tx.send(missing).await.is_err() {
                                    return Ok(()); // connection gone; caller re-runs
                                }
                            }
                        }
                        Some(SyncDown::Update(bytes)) => {
                            self.doc.apply_remote(&bytes)?;
                        }
                        None => {}
                    }
                }
            }
        }
    }
}
