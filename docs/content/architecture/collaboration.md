+++
title = "Real-time collaboration (Loro)"
description = "Offline-first collaborative state: Loro CRDT replicas synced over vox channels — local writes are instant, everything merges, reconnects are deltas."
weight = 58
+++

architect has two state models, and they answer different questions:

| | **Optimistic store** ([optimistic](@/architecture/optimistic.md)) | **CRDT replica** (this page) |
| --- | --- | --- |
| Source of truth | the server | the *document* (every replica holds it) |
| Local write | optimistic patch → reconcile or **rollback** | applied to the local doc, final — concurrent edits **merge** |
| Offline | reads stale cache, writes fail | fully functional; syncs when back |
| Conflicts | server wins | Loro merge semantics (CRDT) |
| Right for | server-owned rows, thin clients | collaborative, offline-first features (Task projects, DAW sessions) |

Both share the same wire (vox), the same fan-out (`PubSub`), the same
phase-match rendering. Pick per feature — the example app runs both
side by side (`Example` is store-backed, `Note` is a CRDT replica).

## One attribute end to end

```rust
#[derive(architect::Entity, ::facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "notes", repo, crdt, form)]
pub struct Note {
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(sortable, fulltext)]
    pub text: String,
    pub author: String,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub updated_at: DateTime<Utc>,
}
```

`#[architect(crdt)]` (user-crate feature `crdt = ["dep:crdt"]`) emits:

- the **`EntityCrdt` codec** — field ↔ `LoroMap` mapping inferred from
  the field types (`Uuid`, `String`, `bool`, ints, `DateTime<Utc>`,
  `Vec<String>`, `Option` of each), with the same `on_create` /
  `on_update` / `exclude` policy the SeaORM path uses;
- **`NoteRepoLoro`** — a Loro-backed implementation of the same
  `NoteRepo` trait the SQL storage implements, over any `CrdtDoc`;
- with the `atom` feature too (convention: `atom = ["architect/atom",
  "crdt?/dioxus"]`): **`use_note_crdt_list()` / `use_note_crdt(id)`**
  returning the same `AtomResult` phases pages already `match`, and
  **`use_note_crdt_actions()`** — create/update/delete that write the
  local replica directly. No optimistic machinery, no rollback arm:
  writes are instant and *final*, concurrent edits merge.

## The stack

`features/crdt/crdt` is the local-first layer: one [`CrdtDoc`] per collaboration
boundary (a project, a workspace, a DAW session) wrapping a `LoroDoc` +
pluggable `Persistence`, and typed `LoroRepo<E>` CRUD views per entity.
`crdt::sync` (feature `vox`) is the transport:

```text
 client A                      server                       client B
 ┌────────────┐   sync(vv,up,down)  ┌──────────────┐  sync   ┌────────────┐
 │ CrdtDoc    │ ──────────────────▶ │ canonical    │ ◀────── │ CrdtDoc    │
 │ (replica)  │  ◀── backlog ────── │ CrdtDoc      │ ──────▶ │ (replica)  │
 │ local ops ─┼──── up channel ───▶ │ + Persistence│ ─down─▶ │            │
 │            │ ◀─── down channel ──│ + PubSub     │         │            │
 └────────────┘                     └──────────────┘         └────────────┘
```

- **`DocSync`** — one `#[vox::service]` method. The client sends its
  Loro **version vector** plus an up-channel and a down-sink; the
  server answers with exactly the missing history
  (`ExportMode::Updates { from }` — an offline week is a delta, not a
  re-download) and **returns its own version vector**, so the client
  pushes back anything the *server* is missing. Catch-up is
  bidirectional: a replica that edited offline, restarted (in-memory
  outbox gone), and reconnected still delivers its history.
- **`DocSyncHost`** — the server: canonical doc + unbounded fan-out
  (update bytes are never dropped). Relayed updates are persisted
  explicitly (imports don't fire the local-update subscription).
  `.with_compaction(n)` folds the update log into a snapshot every `n`
  updates so storage stays bounded; `.with_shallow_bootstrap()` serves
  fresh joiners a shallow snapshot instead of full history.
- **`SyncedDoc`** — the client driver: outbox-buffers local updates
  while offline, runs one `sync` session per connection, merges
  everything that comes down. `run()` is a plain future — spawn it with
  tokio, `dioxus::spawn`, or wasm-bindgen.
- **`DocPresence` / `PresenceHost` / `PresencePeer`** — who's here,
  over Loro's `EphemeralStore` (timestamp-LWW, auto-expiring) and a
  *sliding* PubSub: presence is state-shaped and droppable, unlike doc
  updates. Late joiners get the current picture from the host's mirror
  on attach; peers re-announce their keys on reconnect.

## Dioxus hooks

```rust
// app root — one synced replica + presence per collaboration boundary:
crdt::use_synced_doc(COLLAB_DOC_ID);
crdt::use_presence_channel(COLLAB_DOC_ID, 30_000);

// a page (or use the derive's entity-bound wrappers):
let handle = crdt::use_doc_handle();        // .status(): Connecting | Live | Offline
let notes  = use_note_crdt_list();          // AtomResult — same match as everywhere
let actions = use_note_crdt_actions();      // .create/.update/.delete → local replica
let presence = crdt::use_presence();        // .set(key, value), .states()
```

`use_synced_doc` owns the replica, bridges the doc's change
subscription to a revision `Signal` (every committed change — local or
a peer's — re-renders the readers), and keeps one sync session alive
against the app's shared `Connection<Caller>`, retrying with the
version vector so every reconnect is a delta. The doc is usable
*immediately*, online or not. `use_synced_doc_with(doc_id, || async {
CrdtDoc::open(doc_id, persistence) })` makes the replica itself survive
restarts:

- **desktop / server**: `FilePersistence` (snapshot + update log under
  a directory, write-then-rename, compaction prunes the log);
- **browser**: `IndexedDbPersistence` (feature `indexeddb`) — same
  layout in IndexedDB object stores.

## Many documents (`DocRegistry`)

`DocSyncHost` / `PresenceHost` each serve exactly one `doc_id` — but a
product like Task wants **one doc per project**, with projects coming
and going at runtime. `crdt::registry::DocRegistry` implements the same
`DocSync` + `DocPresence` service traits, so *one* mounted dispatcher
pair serves every doc: on `sync(doc_id, …)` / `presence(doc_id, …)` it
looks up — or lazily opens — the per-doc host and delegates. All the
single-doc invariants (snapshot-then-changes attach, durable relay,
weak-handle subscriptions, unbounded doc fan-out) are inherited by
composition.

```rust
let registry = DocRegistry::new(move |doc_id| {
    let root = data_dir.clone();
    // the factory owns per-doc persistence policy — FilePersistence
    // already namespaces by doc id under its root; a DB backend keys
    // its rows by doc_id the same way
    Box::pin(async move { CrdtDoc::open(doc_id, FilePersistence::new(root)).await })
})
.with_compaction(64)                                  // per created host
.with_shallow_bootstrap()                             // per created host
.with_presence_timeout(30_000)                        // per created PresenceHost
.with_admission(|doc_id| known.contains(&doc_id))     // gate before open/serve
.with_idle_eviction(Duration::from_secs(900));        // compact + drop idle docs

router = router
    .with(doc_sync_service_descriptor(), DocSyncDispatcher::new(registry.clone()))
    .with(doc_presence_service_descriptor(), DocPresenceDispatcher::new(registry));
```

- **Factory** — `Fn(Uuid) -> Pin<Box<dyn Future<Output = Result<CrdtDoc,
  PersistError>> + Send>>`: the caller decides persistence per doc.
  One `tokio::Mutex` guards the doc map across the factory call, so two
  concurrent first-attaches can't double-open the same persistence.
- **Admission** — `Fn(Uuid) -> bool`, checked before a doc is opened
  *or* served; rejected ids fail with `UnknownDoc` (indistinguishable
  from a doc that doesn't exist). This is coarse server-side *scoping*,
  not authorization — the hook sees only the doc id, not who's asking;
  real per-user authz needs caller identity threaded through the
  transport (future work: vox middleware context).
- **Idle eviction** — a sweeper compacts and drops docs with zero live
  sessions and no activity for the configured window; the next attach
  reopens from persistence (one snapshot import, thanks to the
  pre-evict compaction). The eviction-vs-in-flight-attach race is
  closed by re-checking session counts under the same creation lock
  every attach holds from lookup through completion.

Client side, the **keyed hooks** pair with the registry: where
`use_synced_doc` / `use_presence_channel` provide a single unkeyed
context (two simultaneous docs would collide), `use_synced_doc_keyed(doc_id)`
and `use_presence_channel_keyed(doc_id, timeout_ms)` *return* their
handles instead — a component holds one per doc. The unkeyed hooks are
now thin wrappers that delegate to the keyed ones and provide context.

## Server wiring (the example app's shape)

```rust
// once per process, like the evented repo wrapper — a DocRegistry over
// file persistence (one subdir per doc), notes doc opened eagerly:
let collab = Collab::open("./collab-data").await?;
// per connection (clones are cheap over the same doc map):
router = collab.mount(router);                        // DocSync + DocPresence
```

Proven end-to-end by `features/crdt/crdt/tests/sync_convergence.rs` (in-process:
bidirectional convergence, late joiner, offline-restart push-back,
shutdown teardown), `features/crdt/crdt/tests/registry.rs` (two docs through one
dispatcher pair, presence isolation, admission gate, evict-then-reopen)
and `app-tests-e2e` (`notes_replicas_converge_over_websocket`,
`presence_propagates_between_peers`, `two_docs_sync_through_one_registry`
— real WebSocket, real axum server, all through the registry).
The example app's **Collab** page is the live showcase: open it in two
windows, type, kill the server, keep typing, restart it.

## Loro gotchas the framework already encodes

- **Subscription closures never own a strong `CrdtDoc`.** Loro fires
  callbacks synchronously under internal locks; if a closure is the
  doc's last owner, the final drop happens inside unsubscribe and
  deadlocks. Hold `CrdtDoc::downgrade()` (`WeakCrdtDoc`) and send work
  through a channel — see `DocSyncHost`'s compaction worker.
- **Imports don't fire `subscribe_local_update`** — a host applying
  relayed updates persists them explicitly
  (`CrdtDoc::apply_remote_durable`).
- **Events fire synchronously during `commit()`/`import()`** — hook
  callbacks only push into channels; signal writes happen in Dioxus
  tasks.

## Text fields are LWW today

A `String` field on a `#[architect(crdt)]` entity is stored as a plain
`LoroValue::String` in the row's `LoroMap` — **last-writer-wins on the
whole string**. If two peers edit the same text field concurrently, the
merge keeps one peer's entire value and the other's edit is **lost**
(the row converges, but not character-by-character). That's fine for
short, rarely-contended fields (a title, an author name); it is the
wrong tool for collaborative prose.

The planned fix is an opt-in attribute — `#[architect(crdt(text))]` —
mapping the field to a child `LoroText` container instead, so
concurrent edits merge at character level. The runtime support already
exists (`crdt::codec::{text_child, read_text, apply_text_ops,
apply_text_diff}`, including migration from legacy LWW strings via
`read_text_with_migration`); the derive just doesn't emit it yet. Until
it lands, treat every derived text field as LWW and reach for the codec
helpers directly where character-level merging matters.

## What this means for Task and DAW

- **Task** (local-first, multi-device): one doc per project — a
  `DocRegistry` on the server, keyed hooks on the client; tasks/
  cycles/milestones as `#[architect(crdt)]` entities in it. Devices
  work offline and merge on reconnect; file/IndexedDB persistence makes
  restarts free.
- **DAW**: a session doc for collaborative arrangement state (markers,
  regions, routing), with the REAPER extension holding a replica
  in-process — host edits broadcast to web UIs and vice versa. Live
  *signal* data (peaks, playhead) stays on `PubSub` event streams; CRDT
  is for *document* state.
