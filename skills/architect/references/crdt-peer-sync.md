# crdt: documents, text, schema, hub sync, `crdt::peer`, DocRegistry

Crate `crdt` (`features/crdt/crdt`); guides `docs/content/architecture/collaboration.md` and `docs/content/architecture/peers.md`. Feature gates: `vox` (sync, registry, peer), `dioxus` (hooks), `turso`, `indexeddb`.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `CrdtDoc` | owns a `LoroDoc` + `Persistence`; local-update writer task | `features/crdt/crdt/src/lib.rs` `CrdtDoc` (`open`, `ephemeral`, `downgrade`, `repo`, `compact`, `apply_remote`, `apply_remote_durable`), `WeakCrdtDoc` |
| `LoroRepo` / `EntityCrdt` | typed row CRUD over one root map; text ops | `features/crdt/crdt/src/lib.rs` `LoroRepo`, `EntityCrdt`, `apply_text_ops` |
| Derives | `#[architect(crdt)]` + field `crdt(text)`; `entity_crdt!` | `crates/architect-derive/src/lib.rs`; `features/crdt/crdt-derive/src/lib.rs` `entity_crdt` |
| Codec and text | scalar codecs; `TextOp`; `write_text` (minimal diff), `apply_text_diff`, `upgrade_to_text`, `read_text_or_legacy` | `features/crdt/crdt/src/codec.rs` `write_text`, `upgrade_to_text`, `read_text_or_legacy` |
| Schema versions | stamp + ordered, per-step-committed migrations | `features/crdt/crdt/src/schema.rs` `schema_version`, `set_schema_version`, `migrate`, `Migration`, `SchemaStamp` |
| Persistence | trait + File, Turso, IndexedDB, InMemory, SeaORM | `features/crdt/crdt/src/persistence.rs` `Persistence`; `features/crdt/crdt/src/fs.rs` `FilePersistence`; `features/crdt/crdt-seaorm/src/lib.rs` |
| Hub sync | `DocSync` call = session; version-vector catch-up; fan-out | `features/crdt/crdt/src/sync.rs` `DocSync`, `DocSyncHost`, `SyncedDoc` |
| Presence | `EphemeralStore` presence; sliding fan-out | `features/crdt/crdt/src/sync.rs` `DocPresence`, `PresenceHost`, `PresencePeer`, `PresenceDriver` |
| `DocRegistry` | many docs on one dispatcher pair; admission, authorizer, eviction | `features/crdt/crdt/src/registry.rs` `DocRegistry`, `with_admission`, `with_authorizer`, `with_idle_eviction` |
| `crdt::peer` | pairwise exchange, `DocPeer` lane, `SelfLane`, `pull`, lineage check | `features/crdt/crdt/src/peer.rs` `DocPeer`, `DocPeerHost`, `SelfLane`, `exchange`, `pull`, `is_foreign_lineage` |
| Hooks | Dioxus bridge: synced doc, crdt list/entry, presence | `features/crdt/crdt/src/hooks.rs` `use_synced_doc_keyed_with`, `use_crdt_list`, `use_crdt_entry` |
| iroh seam | peer lookup modes; `Principal::Host` identity | `crates/architect/src/iroh_link.rs` `PeerLookup`; `features/permissions/src/lib.rs` `Principal` |

## Which model for which datum

| Datum | Model |
|---|---|
| Multi-writer doc; concurrent edits must both survive; one doc per collaboration boundary | `CrdtDoc` + `LoroRepo<E>` |
| Prose or any long text edited concurrently | `#[architect(crdt(text))]` on the `String` field (a `LoroText` child written by minimal diff), or an explicit `LoroText` via `apply_text_ops` |
| Short scalar fields (title, done, id refs) | plain `#[architect(crdt)]` / `entity_crdt!` fields: whole-value LWW per field |
| Who is here, cursors, selections | presence (`EphemeralStore`): never in the doc; expires |
| Devices share docs with no hub | `crdt::peer` over `DocRegistry` plus Host admission |
| A canonical server exists and clients attach | `DocSyncHost` / `DocRegistry` + `SyncedDoc` (hub) |
| Server-owned rows the server validates | not CRDT: optimistic `Store` (`references/offline-atom.md`) |

## Recipe A: pairwise, no hub (mirrors `features/crdt/crdt/tests/peer.rs`)

1. Node registry: `DocRegistry::new(factory).with_authorizer(|who, doc_id| ..)`. The factory is `FilePersistence` (or `CrdtDoc::ephemeral()` in tests).
2. Serve: `DocPeerHost::new(reg.clone(), OpenDocs(reg.clone()), |endpoint| admitted.contains(endpoint))`; mount `LayerRouter::new().with(doc_peer_service_descriptor(), DocPeerDispatcher::new(host))`, wrapped by `PermissionsGate::new(..).permit(doc_peer_service_descriptor(), &PEER_PERMITS).wrap(router)`. `PEER_PERMITS` (the test's table): `list` read, `sync` write, both on `docs/**`.
3. Identity: the gate's resolver must yield `Principal::Host { endpoint }` from the proved iroh connection (`IrohLink::remote_id()`); the host app maps it.
4. Create the document once on one node, then carry it to the others. Separately created docs with one id are refused as foreign lineage.
5. Pull per node, on a timer: `SelfLane::open(reg.clone(), Principal::Service { name })`, a `DocPeerClient` over the link, `OpenDocs(reg).held()` digests, then `pull(&local, &peer, &ours, per_doc)` → `PullReport`. Single document: `exchange(doc_id, &local, &PeerLane { client, name })`.
6. Convergence is by repeated sweeps, not one call; each pull moves both directions. Proof: proptest `any_edit_order_converges`.

## Recipe B: hub client (mirrors `features/crdt/crdt/tests/sync_convergence.rs`)

- Keyed hook, one replica per doc: `use_synced_doc_keyed_with(doc_id, || CrdtDoc::open(doc_id, FilePersistence::new(root)))`, then `handle.repo::<E>()`.
- `use_crdt_list` / `use_crdt_entry` read the unkeyed context `DocHandle`, which collides across docs: use keyed + `repo` for more than one doc.
- Server: `DocRegistry` with `DocSyncDispatcher` and `DocPresenceDispatcher` on one router.
- Test oracles: `replicas_converge_and_late_joiners_catch_up`, `offline_history_pushes_back_after_restart` (`features/crdt/crdt/tests/sync_convergence.rs`); `features/crdt/crdt/tests/registry.rs`.

## Recipe C: version a document

```rust
pub const SCHEMA: u32 = 2;
pub const MIGRATIONS: &[Migration] = &[
    Migration { to: 1, name: "seed roots", apply: v1_seed_roots },
    Migration { to: 2, name: "notes → LoroText", apply: v2_notes_to_text },
];
let report = crdt::schema::migrate(doc.loro(), SCHEMA, MIGRATIONS)?;
```

- The stamp lives at `__architect_meta.schema_version`; an unstamped doc is version 0. A doc that already stamps itself elsewhere names it: `SchemaStamp::new("meta", "schema").migrate(...)` (Session's layout).
- Steps run one version at a time and commit after each; a gap is an error. A doc stamped newer than the app is refused (`RepoError::Conflict`): no downgrade.
- Steps must be idempotent (two offline peers both migrate; both ops merge). Seed legacy strings with `codec::upgrade_to_text` (a plain container, LWW on creation), never by inserting into a mergeable container, or the seed appears twice.
- Run it after the initial load/sync: migrating an empty joiner stamps it.

## Pitfalls

- **Loro subscription closures must hold `WeakCrdtDoc`.** A strong `CrdtDoc` dropped inside Loro's unsubscribe path deadlocks.
- **Imports don't fire local-update subscriptions.** `apply_remote` is not persisted; relaying hosts use `apply_remote_durable`. A hub client's `FilePersistence` keeps only its own edits and re-learns the rest by version-vector catch-up.
- **Status flashes Live before attach succeeds** in the hook's drive loop; a failed attach then sets `Offline`.
- **Admission sees only the doc id; the authorizer sees the caller** (`permissions_gate::caller()`). `None` means no identity: refuse. Both refusals answer `SyncError::UnknownDoc`, so a peer cannot tell forbidden from absent (by design). `DocRegistry::doc()` skips the authorizer (server-side reads).
- **`DocPeerHost` admits only `Principal::Host`.** Users and services are refused on `list` and `sync`.
- **Foreign lineage is refused, never merged.** `pull` counts it in `foreign`; someone must pick one copy. A timed-out exchange is `failed`, not `foreign`; retry next sweep. Only a non-empty digest equal to ours is skipped.
- **Permission denials are refusals, not drops** (`InvalidPayload("permission denied: ...")` → `Refused`).
- **Eviction breaks held handles:** writes reach persistence but not live replicas. Enable `with_idle_eviction` only if handles are re-fetched per use.
- **Presence:** never call `sweep()` in a render (it fires subscribers → render loop). Presence fan-out is sliding(64) and drops; CRDT updates use unbounded `PubSub`, because a dropped update is a lost edit.
- **Persistence:** compaction is not atomic by default (snapshot, then prune); `FilePersistence` appends with plain writes, so a crash mid-write can leave a truncated `.loro`; SeaORM compacts in one transaction and sequences by wall-clock nanos; Turso sequences `MAX(seq)+1` under one connection lock.
- **Text positions are Unicode scalars, not UTF-16:** translate at the DOM boundary. `apply_text_diff` is one prefix/suffix edit. Create the text container once, then sync before forking edits. `read_text_with_migration` uses a mergeable container: prefer `upgrade_to_text` for concurrent upgrades.
- **`#[architect(crdt)]` needs `repo` and a `Uuid` pk.** Without `crdt(text)` a `String` is whole-string LWW.
- **wasm:** registry, peer and the sync host are native only; client `SyncedDoc` and hooks compile on wasm; IndexedDB is wasm + `indexeddb`; Turso is native + `turso`; `Persistence` is `?Send` on wasm.
- **Hub and pairwise are different topologies.** A node that is both uses one registry for both. There is no gossip: pairwise sweeps only.

## iroh seam

- `PeerLookup`: `N0` (default), `N0AndLan`, `Lan`; the last two need `iroh-lan` (native only). `Lan` turns relays off and publishes nothing: a LAN-only node cannot reach internet peers.
- `bind_endpoint` is `N0`; `bind_endpoint_with` takes a lookup; `EngineHost::iroh_lookup` sets it. Dial by bare id with `iroh_link::connect(endpoint, remote)`. It redials a restarted peer once and reports `DialErrorKind::PeerUnreachable` when that fails too (`crates/architect/src/iroh_link.rs` `connect_with`).
- Test: `crates/architect/tests/iroh_lan.rs` (needs a multicast interface); manual two-machine check `crates/architect/examples/iroh_lan.rs`.
