//! Local-first source-of-truth layer for architect-style features.
//!
//! Shape:
//!
//! ```text
//! CrdtDoc                       (one per collaboration boundary)
//!  ├── LoroDoc                  (Loro's CRDT engine — source of truth)
//!  ├── Arc<dyn Persistence>     (where snapshot + update bytes land)
//!  └── repo::<E>() -> LoroRepo<E>
//!                               (typed view over a root container,
//!                                satisfies the feature's <Name>Repo)
//! ```
//!
//! Per-entity glue lives behind the [`EntityCrdt`] trait, which the
//! feature crate implements for its wire struct. The
//! [`entity_crdt!`] macro emits the entire impl + the `*RepoLoro`
//! newtype + the forward `*Repo` impl from a one-block declaration
//! — see its docstring.
//!
//! The `LoroRepo<E>` type is generic. The `<Name>Repo` trait that
//! architect emits is per-entity. Bridging the two is a one-line
//! forwarder block in the feature crate (`impl ExampleRepo for
//! LoroRepo<Example>`), kept thin so the macro can emit it later.

use std::marker::PhantomData;
use std::sync::Arc;

pub use architect;
pub use crdt_derive::entity_crdt;
pub use loro;

// Real-time, offline-first sync over vox (`DocSync` + `DocSyncHost` +
// `SyncedDoc`, plus the presence channel). Gated: the core stays
// transport-free.
#[cfg(feature = "vox")]
pub mod sync;

// Multi-doc registry: one mounted DocSync/DocPresence dispatcher pair
// serving any number of docs, opened on demand. Server-side (it manages
// native-only hosts), so it's gated on the target too.
#[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
pub mod registry;
#[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
pub use registry::DocRegistry;

// Dioxus hooks (`use_synced_doc`, `use_crdt_list`, `use_crdt_entry`,
// `use_presence_channel`) — the client-side bridge from a syncing
// replica to `AtomResult` phase rendering. Pulls Dioxus via
// architect's `atom` feature, so it's its own gate.
#[cfg(feature = "dioxus")]
pub mod hooks;
#[cfg(feature = "dioxus")]
pub use hooks::{
    DocHandle, Presence, SyncStatus, use_crdt_entry, use_crdt_list, use_doc_handle, use_doc_slot,
    use_presence, use_presence_channel, use_presence_channel_into, use_presence_channel_keyed,
    use_presence_slot, use_synced_doc, use_synced_doc_into, use_synced_doc_into_with,
    use_synced_doc_keyed, use_synced_doc_keyed_with, use_synced_doc_with,
};

// File-backed persistence — the desktop client's (and a small server's)
// way to make a replica survive restarts. Native only.
#[cfg(not(target_arch = "wasm32"))]
pub mod fs;
#[cfg(not(target_arch = "wasm32"))]
pub use fs::FilePersistence;

// IndexedDB persistence — the browser's way. Wasm only, behind a
// feature (pulls the `idb` bindings).
#[cfg(all(target_arch = "wasm32", feature = "indexeddb"))]
pub mod indexeddb;
#[cfg(all(target_arch = "wasm32", feature = "indexeddb"))]
pub use indexeddb::IndexedDbPersistence;

/// Re-exports for awareness / cursor-sync work. `EphemeralStore`
/// is the Loro primitive for ephemeral state (remote cursors,
/// presence) — timestamp-LWW, partial-update encoding, stale-
/// peer timeouts. Lives in `loro_internal` because the public
/// `loro` crate doesn't expose it.
pub mod awareness {
    pub use loro_internal::awareness::{EphemeralStore, EphemeralStoreEvent};
}

use architect::{Filter, Page, RepoError, Sort, SortOrder};
use loro::{Container, LoroDoc, LoroMap, ValueOrContainer};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

pub mod codec;
pub mod in_memory;
pub mod persistence;

pub use persistence::{PersistError, Persistence};

// ── EntityCrdt trait ──────────────────────────────────────────────────
//
// The seam between `LoroRepo<E>` (entity-agnostic) and the feature's
// wire struct. Every method here is a pure function over a LoroMap or
// the wire types — there's no doc state to thread through, so the
// trait stays trivially testable.
//
// Today the feature crate writes this by hand. Tomorrow
// `architect::Entity` emits it next to the SeaORM bridge: same field
// attrs (`primary_key`, `sortable`, `on_create`) drive both.

/// Per-feature marker + codec for entity ↔ LoroMap.
///
/// `EntityCrdt` is implemented on a zero-sized marker type local to
/// the feature crate (e.g. `pub struct ExampleEntity;`), not directly
/// on the wire struct from `-proto`. The marker is what makes the
/// orphan rules happy: the wire struct lives in `-proto`, the trait
/// lives in `features/crdt/crdt`, neither is local to `-crdt` — but the marker
/// is, so `impl EntityCrdt for <Marker>` is allowed, and
/// `impl <Name>Repo for LoroRepo<Marker>` is too (local type wrapped
/// before any uncovered type param).
pub trait EntityCrdt: Send + Sync + 'static {
    /// The actual wire struct from `-proto`.
    type Wire: Send + 'static;
    /// Wire payload for `create` (no id, no `on_create` fields).
    type Create: Send + 'static;
    /// Wire payload for `update` (Option per non-pk field).
    type Update: Send + 'static;
    /// Wire payload for `list` — typically `{Wire}List` emitted by
    /// the architect macro. Built via `build_list`.
    type List: Send + 'static;

    /// Root container key under the LoroDoc. One per entity type.
    /// e.g. "examples", "projects", "tasks".
    const ROOT: &'static str;

    /// Primary key of a wire row.
    fn id(wire: &Self::Wire) -> Uuid;

    /// Materialize a wire row from a `Create` payload. The feature
    /// owns id-generation policy and timestamping (matching what the
    /// architect macro does for the SeaORM path).
    fn from_create(input: Self::Create) -> Self::Wire;

    /// Encode a wire row into a freshly inserted sub-LoroMap.
    fn encode_into(map: &LoroMap, wire: &Self::Wire) -> Result<(), RepoError>;

    /// Decode a wire row out of an existing sub-LoroMap.
    fn decode_from(map: &LoroMap) -> Result<Self::Wire, RepoError>;

    /// Mutate a sub-LoroMap in place to apply an `Update` payload.
    /// The feature is responsible for refreshing `updated_at` etc.
    fn apply_update(map: &LoroMap, input: Self::Update) -> Result<(), RepoError>;

    /// Sort an in-memory slice by the requested field. Returns
    /// `InvalidInput` for unknown fields — matches the SeaORM impl's
    /// behavior so the trait contract is one shape, two backends.
    fn sort_items(items: &mut [Self::Wire], field: &str, order: SortOrder)
    -> Result<(), RepoError>;

    /// Assemble the entity-specific `List` wire type.
    fn build_list(items: Vec<Self::Wire>, total: u32, page: Page) -> Self::List;
}

// ── CrdtDoc ───────────────────────────────────────────────────────────

/// One collaboration boundary. Owns the LoroDoc, the persistence
/// handle, and the local-update subscription that flushes new bytes
/// back to storage. Construct typed `LoroRepo<E>` handles off it.
///
/// Cheap to clone — internally `Arc`-shared.
#[derive(Clone)]
pub struct CrdtDoc {
    inner: Arc<DocInner>,
}

struct DocInner {
    doc: LoroDoc,
    /// Serializes read-modify-write windows inside `LoroRepo`. Loro
    /// itself merges concurrent local writes safely; this guards the
    /// "fetch sub-map, mutate it" sequence so callers of `update`
    /// don't race.
    write_lock: AsyncMutex<()>,
    /// Persistence handle. Erased so `CrdtDoc` isn't generic over the
    /// backend — keeps the call sites that hand `LoroRepo<E>` to vox
    /// dispatchers identical across deployments.
    persistence: Arc<dyn Persistence>,
    /// Holds the local-update subscription alive for the doc's
    /// lifetime. Dropping it would unsubscribe.
    _local_update_sub: loro::Subscription,
}

impl CrdtDoc {
    /// Open a doc against the given persistence: load any stored
    /// snapshot, replay any updates, wire the subscription that
    /// forwards future commits back to storage.
    ///
    /// Works on both targets. Loro's local-update callback requires
    /// `Send + Sync`, but browser storage handles (IndexedDB) aren't —
    /// so the callback only pushes bytes into a channel, and a single
    /// spawned writer task (tokio native, `spawn_local` wasm) owns the
    /// persistence handle and drains it in commit order.
    pub async fn open<P: Persistence>(doc_id: Uuid, persistence: P) -> Result<Self, PersistError> {
        let persistence: Arc<dyn Persistence> = Arc::new(persistence);
        let doc = LoroDoc::new();

        if let Some(snap) = persistence.load_snapshot(doc_id).await? {
            doc.import(&snap)
                .map_err(|e| PersistError::Backend(format!("import snapshot: {e}")))?;
        }
        for u in persistence.load_updates(doc_id).await? {
            doc.import(&u)
                .map_err(|e| PersistError::Backend(format!("import update: {e}")))?;
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let sub = doc.subscribe_local_update(Box::new(move |bytes| {
            // Writer gone (shutdown) → stop forwarding, keep committing.
            tx.send(bytes.to_vec()).is_ok()
        }));
        let writer = persistence.clone();
        spawn_persist_writer(async move {
            while let Some(bytes) = rx.recv().await {
                if let Err(e) = writer.append_update(doc_id, &bytes).await {
                    tracing::error!(?doc_id, "crdt: failed to persist update: {e}");
                }
            }
        });

        Ok(Self {
            inner: Arc::new(DocInner {
                doc,
                write_lock: AsyncMutex::new(()),
                persistence,
                _local_update_sub: sub,
            }),
        })
    }

    /// Wrap a pre-built `LoroDoc` (e.g. one that already imported a
    /// snapshot from elsewhere). No persistence — for tests, demos,
    /// or callers that own their own I/O.
    pub fn from_loro(doc: LoroDoc) -> Self {
        let persistence: Arc<dyn Persistence> = Arc::new(in_memory::InMemoryPersistence::new());
        let sub = doc.subscribe_local_update(Box::new(|_bytes| true));
        Self {
            inner: Arc::new(DocInner {
                doc,
                write_lock: AsyncMutex::new(()),
                persistence,
                _local_update_sub: sub,
            }),
        }
    }

    /// Construct without persistence — local-only, no snapshot or
    /// update I/O. For tests, demos, and ephemeral workflows.
    pub fn ephemeral() -> Self {
        let persistence: Arc<dyn Persistence> = Arc::new(in_memory::InMemoryPersistence::new());
        let doc = LoroDoc::new();
        // Still wire the subscription — append_update will hit the
        // in-memory backend, which is essentially a no-op for any
        // caller that never reads the persistence handle.
        let sub = doc.subscribe_local_update(Box::new(|_bytes| true));
        Self {
            inner: Arc::new(DocInner {
                doc,
                write_lock: AsyncMutex::new(()),
                persistence,
                _local_update_sub: sub,
            }),
        }
    }

    /// A non-owning handle. **Required** inside loro subscription
    /// callbacks: a callback that owns a strong `CrdtDoc` can become
    /// the doc's last owner, making the doc drop *inside* loro's
    /// unsubscribe path — which re-enters the subscriber registry's
    /// (non-reentrant) lock to drop the doc's own subscription and
    /// deadlocks. Hold a `WeakCrdtDoc` there and `upgrade()` on use.
    pub fn downgrade(&self) -> WeakCrdtDoc {
        WeakCrdtDoc {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Typed handle over the entity's root container. Multiple repos
    /// over different entity types all share this doc — writes
    /// commit together, exports cover the whole boundary.
    pub fn repo<E: EntityCrdt>(&self) -> LoroRepo<E> {
        LoroRepo {
            inner: self.inner.clone(),
            _entity: PhantomData,
        }
    }

    /// Borrow the underlying LoroDoc — for export bundle assembly,
    /// time-travel (`checkout`/`fork`), or custom subscriptions.
    pub fn loro(&self) -> &LoroDoc {
        &self.inner.doc
    }

    /// Capture a full snapshot and write it through `Persistence::compact`,
    /// effectively turning N accumulated updates into one snapshot row.
    /// Call periodically (every N updates, on quiesce, on shutdown).
    pub async fn compact(&self, doc_id: Uuid) -> Result<(), PersistError> {
        let bytes = self
            .inner
            .doc
            .export(loro::ExportMode::Snapshot)
            .map_err(|e| PersistError::Backend(format!("export snapshot: {e}")))?;
        self.inner.persistence.compact(doc_id, &bytes).await
    }

    /// Apply remote update bytes (received over the sync transport).
    /// Loro merges automatically; subscribers see the converged state.
    pub fn apply_remote(&self, bytes: &[u8]) -> Result<(), PersistError> {
        self.inner
            .doc
            .import(bytes)
            .map_err(|e| PersistError::Backend(format!("import remote: {e}")))?;
        Ok(())
    }

    /// [`Self::apply_remote`] + write the bytes through this doc's
    /// persistence. Imports don't fire the local-update subscription
    /// (only local commits do), so a host applying *relayed* updates
    /// must persist them explicitly or they'd vanish on restart.
    pub async fn apply_remote_durable(
        &self,
        doc_id: Uuid,
        bytes: &[u8],
    ) -> Result<(), PersistError> {
        self.apply_remote(bytes)?;
        self.inner.persistence.append_update(doc_id, bytes).await
    }
}

/// Non-owning [`CrdtDoc`] handle — see [`CrdtDoc::downgrade`].
#[derive(Clone)]
pub struct WeakCrdtDoc {
    inner: std::sync::Weak<DocInner>,
}

impl WeakCrdtDoc {
    /// The doc, if it's still alive.
    pub fn upgrade(&self) -> Option<CrdtDoc> {
        self.inner.upgrade().map(|inner| CrdtDoc { inner })
    }
}

// ── LoroRepo<E> ───────────────────────────────────────────────────────
//
// One typed view over the doc's root container for `E`. Cheap to
// clone; multiple clones share the same DocInner. Mutating methods
// hold `DocInner::write_lock` for the duration of read-modify-write
// so updates are atomic from the caller's perspective.

pub struct LoroRepo<E: EntityCrdt> {
    inner: Arc<DocInner>,
    _entity: PhantomData<fn() -> E>,
}

impl<E: EntityCrdt> Clone for LoroRepo<E> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _entity: PhantomData,
        }
    }
}

impl<E: EntityCrdt> LoroRepo<E> {
    pub async fn get(&self, id: Uuid) -> Result<E::Wire, RepoError> {
        self.get_now(id)
    }

    pub async fn list(
        &self,
        page: Page,
        sort: Option<Sort>,
        filter: Option<Filter>,
    ) -> Result<E::List, RepoError> {
        self.list_now(page, sort, filter)
    }

    /// Synchronous `get` — reads never block in Loro (the write lock only
    /// guards read-modify-write windows), so UI code rendering straight
    /// off the local replica can read without an executor.
    pub fn get_now(&self, id: Uuid) -> Result<E::Wire, RepoError> {
        let root = self.inner.doc.get_map(E::ROOT);
        let sub = root
            .get(&id.to_string())
            .and_then(only_map)
            .ok_or(RepoError::NotFound)?;
        E::decode_from(&sub)
    }

    /// Every row, unsorted, unpaged — the render-time read behind
    /// `use_crdt_list`. Local replicas are the UI's working set; pages
    /// sort/slice in memory.
    pub fn items_now(&self) -> Result<Vec<E::Wire>, RepoError> {
        let root = self.inner.doc.get_map(E::ROOT);
        let mut items: Vec<E::Wire> = Vec::with_capacity(root.len());
        let mut decode_err: Option<RepoError> = None;
        root.for_each(|_k, v| {
            if decode_err.is_some() {
                return;
            }
            if let ValueOrContainer::Container(Container::Map(sub)) = v {
                match E::decode_from(&sub) {
                    Ok(e) => items.push(e),
                    Err(e) => decode_err = Some(e),
                }
            }
        });
        match decode_err {
            Some(e) => Err(e),
            None => Ok(items),
        }
    }

    /// Synchronous `list` — the read path behind the async method, exposed
    /// for render-time reads (`use_crdt_list` re-runs it per doc revision).
    pub fn list_now(
        &self,
        page: Page,
        sort: Option<Sort>,
        _filter: Option<Filter>,
    ) -> Result<E::List, RepoError> {
        let mut items = self.items_now()?;

        if let Some(s) = sort {
            E::sort_items(&mut items, &s.field, s.order)?;
        }

        let total = items.len() as u32;
        let size = page.size.max(1) as usize;
        let start = (page.index as usize).saturating_mul(size);
        let items: Vec<E::Wire> = items.into_iter().skip(start).take(size).collect();
        Ok(E::build_list(items, total, page))
    }

    pub async fn create(&self, input: E::Create) -> Result<E::Wire, RepoError> {
        let _g = self.inner.write_lock.lock().await;
        let row = E::from_create(input);
        let root = self.inner.doc.get_map(E::ROOT);
        let sub = root
            .insert_container(&E::id(&row).to_string(), LoroMap::new())
            .map_err(loro_err)?;
        E::encode_into(&sub, &row)?;
        self.inner.doc.commit();
        Ok(row)
    }

    pub async fn update(&self, id: Uuid, input: E::Update) -> Result<E::Wire, RepoError> {
        let _g = self.inner.write_lock.lock().await;
        let root = self.inner.doc.get_map(E::ROOT);
        let sub = root
            .get(&id.to_string())
            .and_then(only_map)
            .ok_or(RepoError::NotFound)?;
        E::apply_update(&sub, input)?;
        self.inner.doc.commit();
        E::decode_from(&sub)
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), RepoError> {
        let _g = self.inner.write_lock.lock().await;
        let root = self.inner.doc.get_map(E::ROOT);
        if root.get(&id.to_string()).is_none() {
            return Err(RepoError::NotFound);
        }
        root.delete(&id.to_string()).map_err(loro_err)?;
        self.inner.doc.commit();
        Ok(())
    }

    /// Borrow the doc this repo lives in. Convenience for tests +
    /// callers that want to drive export/import directly.
    pub fn doc(&self) -> &LoroDoc {
        &self.inner.doc
    }

    /// Apply text ops against a `LoroText` child container on the
    /// entity's sub-map. The fast path for editor keystrokes — bypasses
    /// the full-entity `apply_update` round trip and so doesn't bump
    /// `updated_at`. Callers wanting bookkeeping should follow with
    /// `update`.
    pub async fn apply_text_ops(
        &self,
        id: Uuid,
        key: &str,
        ops: &[crate::codec::TextOp],
    ) -> Result<(), RepoError> {
        let _g = self.inner.write_lock.lock().await;
        let root = self.inner.doc.get_map(E::ROOT);
        let sub = root
            .get(&id.to_string())
            .and_then(only_map)
            .ok_or(RepoError::NotFound)?;
        crate::codec::apply_text_ops(&sub, key, ops)?;
        self.inner.doc.commit();
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Spawn the persistence writer on whatever executor the target has.
/// The future is `!Send` on wasm (it owns the browser storage handle);
/// wasm is single-threaded so `spawn_local` is exactly right.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_persist_writer(fut: impl std::future::Future<Output = ()> + Send + 'static) {
    tokio::spawn(fut);
}
#[cfg(target_arch = "wasm32")]
fn spawn_persist_writer(fut: impl std::future::Future<Output = ()> + 'static) {
    wasm_bindgen_futures::spawn_local(fut);
}

fn only_map(v: ValueOrContainer) -> Option<LoroMap> {
    match v {
        ValueOrContainer::Container(Container::Map(m)) => Some(m),
        _ => None,
    }
}

fn loro_err<E: std::fmt::Display>(e: E) -> RepoError {
    RepoError::Internal(format!("loro: {e}"))
}
