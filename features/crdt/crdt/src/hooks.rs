//! Dioxus hooks for collaborative docs — the client-side bridge from a
//! syncing [`CrdtDoc`] replica to the same `AtomResult` phases pages
//! already `match`.
//!
//! The shape mirrors the optimistic-store layer, with one deliberate
//! difference: there is **no optimistic machinery**. Writes go straight
//! into the local replica (instant, final, offline-capable), the doc's
//! change subscription bumps a revision `Signal`, and every reader
//! re-reads. Remote edits arrive through [`SyncedDoc`] and bump the same
//! revision — local and remote changes render through one path.
//!
//! ```ignore
//! // app root — one synced replica per collaboration boundary:
//! use_synced_doc(COLLAB_DOC_ID);
//!
//! // a page (typically via the derive's entity-bound wrappers):
//! match use_crdt_list::<NoteEntity>() {
//!     Success(notes) => rsx! { for n in notes { NoteRow { note: n } } },
//!     Loading => rsx! { Spinner {} },
//!     Error { error, .. } => rsx! { "{error}" },
//!     _ => rsx! {},
//! }
//! ```

use std::sync::Arc;

use architect::dioxus::prelude::*;
use architect::{AtomResult, Duration, RepoError, use_connection};
use uuid::Uuid;

use crate::sync::{DocPresenceClient, DocSyncClient, PresenceDriver, PresencePeer, SyncedDoc};
use crate::{CrdtDoc, EntityCrdt, LoroRepo, PersistError};

/// Where the replica's sync session currently stands. Purely
/// informational — the local doc is always readable and writable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyncStatus {
    /// No session yet (connection still resolving, or doc still opening).
    Connecting,
    /// A sync session is live — edits flow both ways in real time.
    Live,
    /// The session dropped — edits buffer locally; the driver retries
    /// and the version vector turns reconnection into a delta.
    Offline,
}

/// `Copy` handle to the app's synced doc, provided as context by
/// [`use_synced_doc`]. Feature hooks read the repo + revision off it.
#[derive(Clone, Copy)]
pub struct DocHandle {
    doc: Signal<Option<CrdtDoc>>,
    revision: Signal<u64>,
    status: Signal<SyncStatus>,
}

/// Identity (same underlying signals) — makes the handle a valid prop;
/// reads stay live because the signals themselves are the subscription.
impl PartialEq for DocHandle {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision && self.status == other.status
    }
}

impl DocHandle {
    /// The replica, once open. `None` while an async open (persistent
    /// backends) is still running.
    pub fn doc(&self) -> Option<CrdtDoc> {
        self.doc.read().clone()
    }

    /// Reset the handle to its pre-open state (`None` doc, revision 0,
    /// [`SyncStatus::Connecting`]) — for slot handles
    /// ([`use_doc_slot`]) between drivers: call when tearing one
    /// session down before (or instead of) mounting the next, so no
    /// reader ever observes the previous doc's state. Writes are
    /// skipped when already reset, so calling this from a fresh slot
    /// (or twice) never dirties subscribers.
    pub fn reset(&self) {
        let mut doc = self.doc;
        let mut revision = self.revision;
        let mut status = self.status;
        if doc.peek().is_some() {
            doc.set(None);
        }
        if *revision.peek() != 0 {
            revision.set(0);
        }
        if *status.peek() != SyncStatus::Connecting {
            status.set(SyncStatus::Connecting);
        }
    }

    /// Reactive read of the doc revision — subscribe a memo/component
    /// to *every* committed change, local or remote.
    pub fn revision(&self) -> u64 {
        (self.revision)()
    }

    /// Reactive read of the sync session state (for a status badge).
    pub fn status(&self) -> SyncStatus {
        (self.status)()
    }

    /// Typed repo over the replica, or `None` while opening.
    pub fn repo<E: EntityCrdt>(&self) -> Option<LoroRepo<E>> {
        self.doc().map(|d| d.repo())
    }
}

/// Pull the [`DocHandle`] provided at the app root.
pub fn use_doc_handle() -> DocHandle {
    use_context::<DocHandle>()
}

/// Provide a syncing ephemeral replica of `doc_id` for the component
/// tree. The doc is immediately usable (local-first); a background
/// driver keeps one sync session against the app's shared
/// [`Connection<vox::Caller>`](architect::Connection) alive, retrying
/// with the version vector so every reconnect is a delta.
pub fn use_synced_doc(doc_id: Uuid) -> DocHandle {
    use_synced_doc_with(doc_id, || async { Ok(CrdtDoc::ephemeral()) })
}

/// [`use_synced_doc`] with a custom (possibly async) doc constructor —
/// pass `CrdtDoc::open(doc_id, persistence)` to make the replica itself
/// survive restarts (file on desktop, IndexedDB in the browser).
pub fn use_synced_doc_with<F, Fut>(doc_id: Uuid, open: F) -> DocHandle
where
    F: FnOnce() -> Fut + 'static,
    Fut: std::future::Future<Output = Result<CrdtDoc, PersistError>> + 'static,
{
    let handle = use_synced_doc_keyed_with(doc_id, open);
    use_context_provider(|| handle)
}

/// Keyed variant of [`use_synced_doc`]: same machinery (replica + sync
/// session + revision signal), but the [`DocHandle`] is **returned, not
/// provided as context** — so a component can hold several docs at once
/// (the unkeyed context hooks would collide on the single `DocHandle`
/// context). Pair with a [`DocRegistry`](crate::registry::DocRegistry)
/// on the server, which serves any `doc_id` over one mounted dispatcher.
///
/// `doc_id` is captured on first render; to switch documents, remount
/// the component (give it a `key`).
///
/// ```ignore
/// let project = use_synced_doc_keyed(project_doc_id);
/// let inbox = use_synced_doc_keyed(inbox_doc_id);
/// ```
pub fn use_synced_doc_keyed(doc_id: Uuid) -> DocHandle {
    use_synced_doc_keyed_with(doc_id, || async { Ok(CrdtDoc::ephemeral()) })
}

/// [`use_synced_doc_keyed`] with a custom (possibly async) doc
/// constructor — see [`use_synced_doc_with`].
pub fn use_synced_doc_keyed_with<F, Fut>(doc_id: Uuid, open: F) -> DocHandle
where
    F: FnOnce() -> Fut + 'static,
    Fut: std::future::Future<Output = Result<CrdtDoc, PersistError>> + 'static,
{
    let handle = use_doc_slot();
    use_synced_doc_into_with(handle, doc_id, open);
    handle
}

/// Create an **empty slot** [`DocHandle`] owned by the *current* scope
/// — no doc is opened and no session started until a driver fills it.
///
/// This is the ownership half of the split keyed API. Dioxus `Copy`
/// handles (signals) are owned by the scope that creates them; a
/// handle created inside a keyed/conditional child and captured by
/// longer-lived closures (a decoration source, an event callback, a
/// parent's render) is read **after its owner dropped** the moment the
/// child remounts — the classic "Copy value used in a scope that is
/// not a descendant of the owner" violation. The fix dioxus itself
/// prescribes is hoisting: create the signals in the outliving scope.
///
/// So: call `use_doc_slot()` in the stable (page) scope, hand the
/// handle to everything that captures it, and let a keyed child drive
/// it with [`use_synced_doc_into`] — the *session* lifecycle stays
/// keyed (fresh replica per remount) while the *signals* live as long
/// as every reader. A fresh slot reads as `None` doc / revision 0 /
/// [`SyncStatus::Connecting`]; see [`DocHandle::reset`].
pub fn use_doc_slot() -> DocHandle {
    DocHandle {
        doc: use_signal(|| None::<CrdtDoc>),
        revision: use_signal(|| 0u64),
        status: use_signal(|| SyncStatus::Connecting),
    }
}

/// Drive a [`use_doc_slot`] slot: open an ephemeral replica of
/// `doc_id` and keep one sync session alive for the *calling* scope's
/// lifetime, writing doc/revision/status **into** the slot's
/// parent-owned signals. The slot is [`reset`](DocHandle::reset) on
/// mount, so a remounted driver never lets readers see the previous
/// doc's revision/status. `doc_id` is captured on first render —
/// remount (`key:`) to switch documents.
pub fn use_synced_doc_into(handle: DocHandle, doc_id: Uuid) {
    use_synced_doc_into_with(handle, doc_id, || async { Ok(CrdtDoc::ephemeral()) });
}

/// [`use_synced_doc_into`] with a custom (possibly async) doc
/// constructor — see [`use_synced_doc_with`].
pub fn use_synced_doc_into_with<F, Fut>(handle: DocHandle, doc_id: Uuid, open: F)
where
    F: FnOnce() -> Fut + 'static,
    Fut: std::future::Future<Output = Result<CrdtDoc, PersistError>> + 'static,
{
    // A slot can carry a previous driver's state — clear it before any
    // effect of this mount generation can observe it (no-op writes are
    // skipped, so a fresh slot stays untouched).
    use_hook(|| handle.reset());
    let DocHandle {
        doc: doc_sig,
        revision,
        status,
    } = handle;
    let mut revision = revision;
    let conn = use_connection::<vox::Caller>();
    let mut open_slot = use_hook(|| CopyValue::new(Some(open)));

    use_future(move || async move {
        let Some(open) = open_slot.write().take() else {
            return;
        };
        let mut doc_sig = doc_sig;
        let mut status = status;
        let doc = match open().await {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(?doc_id, "crdt: failed to open replica: {e}");
                status.set(SyncStatus::Offline);
                return;
            }
        };

        // Change subscription → revision bump. The callback fires on
        // every commit/import from arbitrary contexts, so it only pushes
        // into a channel; the bump itself happens here, inside the
        // Dioxus task.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let change_sub = doc.loro().subscribe_root(Arc::new(move |_| {
            let _ = tx.send(());
        }));

        // Wire the outbox BEFORE publishing the doc — every local edit
        // from the first render onward must reach the sync session.
        let mut synced = SyncedDoc::new(doc_id, doc.clone());
        doc_sig.set(Some(doc));

        let bump = async move {
            let _keep = change_sub;
            while rx.recv().await.is_some() {
                revision += 1;
            }
        };
        let drive = async move {
            let mut status = status;
            // Exponential backoff with full jitter between failed session
            // attempts (floor 250ms, cap 10s); reset whenever the shared
            // connection re-establishes (generation bump) so a recovered
            // server is consumed immediately. Logs once per state change,
            // never per attempt.
            let mut backoff = architect::Backoff::default();
            let mut last_generation: Option<u64> = None;
            let mut outage_logged = false;
            loop {
                // Re-read the connection every iteration (never capture a
                // caller across attempts) and skip corpses outright —
                // `is_connected()` is false the instant the socket dies,
                // even before the supervisor flips the state.
                let Some(caller) = conn.ready().filter(vox::Caller::is_connected) else {
                    architect::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                let generation = conn.generation_now();
                if last_generation != Some(generation) {
                    backoff.reset();
                    if outage_logged {
                        tracing::info!(
                            ?doc_id,
                            generation,
                            "crdt: connection recovered; reattaching sync session"
                        );
                        outage_logged = false;
                    }
                    last_generation = Some(generation);
                }
                if *status.peek() != SyncStatus::Live {
                    status.set(SyncStatus::Live);
                }
                match synced.run(&DocSyncClient::new(caller)).await {
                    // `Ok` = the session attached and later dropped
                    // (stream ended / send failed): the start of an
                    // outage — one warn, then quiet until recovery.
                    Ok(()) => {
                        tracing::warn!(?doc_id, "crdt: sync session dropped; reconnecting");
                        outage_logged = true;
                    }
                    // `Err` = attach (or apply) failed — usually a dead
                    // caller racing the supervisor. One warn per outage.
                    Err(e) => {
                        if !outage_logged {
                            tracing::warn!(
                                ?doc_id,
                                "crdt: sync session error: {e}; retrying with backoff"
                            );
                            outage_logged = true;
                        } else {
                            tracing::debug!(?doc_id, "crdt: sync reattach failed: {e}");
                        }
                    }
                }
                if *status.peek() != SyncStatus::Offline {
                    status.set(SyncStatus::Offline);
                }
                architect::sleep(backoff.next_delay()).await;
            }
        };
        // Both run for the component's lifetime; the future is dropped
        // (and the session with it) on unmount.
        tokio::select! {
            _ = bump => {},
            _ = drive => {},
        }
    });
}

/// Every row of `E` in the synced doc, re-read on every doc revision.
/// `Loading` only while an async open is still running — a local
/// replica never waits on the network to render.
pub fn use_crdt_list<E: EntityCrdt>() -> AtomResult<Vec<E::Wire>, RepoError>
where
    E::Wire: Clone,
{
    let handle = use_doc_handle();
    let _ = handle.revision(); // subscribe to changes
    let Some(repo) = handle.repo::<E>() else {
        return AtomResult::Loading;
    };
    match repo.items_now() {
        Ok(items) => AtomResult::Success(items),
        Err(error) => AtomResult::Error { error, last: None },
    }
}

/// One row by id, re-read on every doc revision. `Error(NotFound)`
/// when the row doesn't exist (yet — a peer may still be typing it).
pub fn use_crdt_entry<E: EntityCrdt>(id: Uuid) -> AtomResult<E::Wire, RepoError>
where
    E::Wire: Clone,
{
    let handle = use_doc_handle();
    let _ = handle.revision();
    let Some(repo) = handle.repo::<E>() else {
        return AtomResult::Loading;
    };
    match repo.get_now(id) {
        Ok(item) => AtomResult::Success(item),
        Err(error) => AtomResult::Error { error, last: None },
    }
}

/// `Copy` handle to the doc's presence channel, provided as context by
/// [`use_presence_channel`].
#[derive(Clone, Copy)]
pub struct Presence {
    peer: Signal<Option<PresencePeer>>,
    revision: Signal<u64>,
}

/// Identity (same underlying signals) — makes the handle a valid prop.
impl PartialEq for Presence {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
    }
}

impl Presence {
    /// Publish this client's state under `key` (conventionally its
    /// client id). No-op until the channel is wired (first render).
    pub fn set(&self, key: &str, value: impl Into<loro::LoroValue>) {
        if let Some(peer) = &*self.peer.read() {
            peer.set(key, value);
        }
    }

    /// Withdraw a key explicitly.
    pub fn delete(&self, key: &str) {
        if let Some(peer) = &*self.peer.read() {
            peer.delete(key);
        }
    }

    /// Everyone's current state, reactive — re-renders on every
    /// presence change, expired peers pruned.
    pub fn states(&self) -> std::collections::HashMap<String, loro::LoroValue> {
        let _ = (self.revision)();
        match &*self.peer.read() {
            Some(peer) => peer.states(),
            None => Default::default(),
        }
    }

    /// Reset the handle to its pre-join state (no peer, revision 0) —
    /// for slot handles ([`use_presence_slot`]) between drivers, the
    /// presence twin of [`DocHandle::reset`]. No-op writes are skipped.
    pub fn reset(&self) {
        let mut peer = self.peer;
        let mut revision = self.revision;
        if peer.peek().is_some() {
            peer.set(None);
        }
        if *revision.peek() != 0 {
            revision.set(0);
        }
    }
}

/// Join `doc_id`'s presence channel for the component tree's lifetime:
/// provides a [`Presence`] handle, keeps one session against the shared
/// connection alive, and re-announces this client's keys on reconnect.
pub fn use_presence_channel(doc_id: Uuid, timeout_ms: i64) -> Presence {
    let presence = use_presence_channel_keyed(doc_id, timeout_ms);
    use_context_provider(|| presence)
}

/// Keyed variant of [`use_presence_channel`]: same machinery, but the
/// [`Presence`] handle is **returned, not provided as context** — hold
/// one per doc when a component joins several docs' presence channels
/// at once. `doc_id` is captured on first render; remount (`key`) to
/// switch.
pub fn use_presence_channel_keyed(doc_id: Uuid, timeout_ms: i64) -> Presence {
    let presence = use_presence_slot();
    use_presence_channel_into(presence, doc_id, timeout_ms);
    presence
}

/// Create an **empty slot** [`Presence`] owned by the *current* scope
/// — nobody joins until a driver fills it. The presence twin of
/// [`use_doc_slot`]: create the handle in the stable (page) scope so
/// closures that capture it (decoration sources, callbacks) never read
/// signals owned by a dropped keyed child; drive it from the keyed
/// child with [`use_presence_channel_into`].
pub fn use_presence_slot() -> Presence {
    Presence {
        peer: use_signal(|| None::<PresencePeer>),
        revision: use_signal(|| 0u64),
    }
}

/// Drive a [`use_presence_slot`] slot: join `doc_id`'s presence
/// channel for the *calling* scope's lifetime, writing the peer +
/// revision **into** the slot's parent-owned signals. The slot is
/// [`reset`](Presence::reset) on mount. `doc_id` is captured on first
/// render — remount (`key:`) to switch.
pub fn use_presence_channel_into(presence: Presence, doc_id: Uuid, timeout_ms: i64) {
    use_hook(|| presence.reset());
    let Presence {
        peer: peer_sig,
        revision,
    } = presence;
    let mut peer_sig = peer_sig;
    let mut revision = revision;
    let conn = use_connection::<vox::Caller>();

    use_future(move || async move {
        let (peer, mut driver): (PresencePeer, PresenceDriver) =
            PresencePeer::new(doc_id, timeout_ms);

        // Any change (local set, remote apply, timeout) → revision bump,
        // bridged through a channel for the same reason as the doc hook.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let change_sub = peer.store().subscribe(Box::new(move |_| {
            let _ = tx.send(());
            true
        }));
        let peer_for_sweep = peer.clone();
        peer_sig.set(Some(peer));

        let bump = async move {
            let _keep = change_sub;
            while rx.recv().await.is_some() {
                revision += 1;
            }
        };
        let drive = async move {
            // Same recovery contract as the doc-sync drive loop: re-read
            // the connection each attempt, skip dead callers, back off
            // exponentially with full jitter (reset on generation bump),
            // and log once per state change — not once per second.
            let mut backoff = architect::Backoff::default();
            let mut last_generation: Option<u64> = None;
            let mut outage_logged = false;
            loop {
                let Some(caller) = conn.ready().filter(vox::Caller::is_connected) else {
                    architect::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                let generation = conn.generation_now();
                if last_generation != Some(generation) {
                    backoff.reset();
                    if outage_logged {
                        tracing::info!(
                            ?doc_id,
                            generation,
                            "presence: connection recovered; reattaching"
                        );
                        outage_logged = false;
                    }
                    last_generation = Some(generation);
                }
                match driver.run(&DocPresenceClient::new(caller)).await {
                    Ok(()) => {
                        tracing::warn!(?doc_id, "presence session dropped; reconnecting");
                        outage_logged = true;
                    }
                    Err(e) => {
                        if !outage_logged {
                            tracing::warn!(
                                ?doc_id,
                                "presence session error: {e}; retrying with backoff"
                            );
                            outage_logged = true;
                        } else {
                            tracing::debug!(?doc_id, "presence reattach failed: {e}");
                        }
                    }
                }
                architect::sleep(backoff.next_delay()).await;
            }
        };
        // Expiry sweep — prune vanished peers on a timer. Pruning fires
        // store events (→ bump → re-render), so it must live HERE, in a
        // task, never in a render path like `states()`.
        let sweep_peer = peer_for_sweep;
        let sweep = async move {
            let period = Duration::from_millis((timeout_ms as u64 / 2).max(1_000));
            loop {
                architect::sleep(period).await;
                sweep_peer.sweep();
            }
        };
        tokio::select! {
            _ = bump => {},
            _ = drive => {},
            _ = sweep => {},
        }
    });
}

/// Pull the [`Presence`] handle provided at the app root.
pub fn use_presence() -> Presence {
    use_context::<Presence>()
}
