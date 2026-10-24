# crdt: documents, text, schema, hub sync, `crdt::peer`, DocRegistry

Crate `crdt` (`features/crdt/crdt`); guides `docs/content/architecture/collaboration.md` and `docs/content/architecture/peers.md`. Feature gates: `vox` (sync, registry, peer), `dioxus` (hooks), `turso`, `indexeddb`.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `CrdtDoc` | owns a `LoroDoc` + `Persistence`; local-update writer task | `features/crdt/crdt/src/lib.rs` `CrdtDoc` (`open`, `open_shared(doc_id, Arc<dyn Persistence>)` for one device store behind many docs, `ephemeral`, `downgrade`, `repo`, `compact`, `apply_remote` (not persisted), `apply_remote_durable`), `WeakCrdtDoc` |
| `LoroRepo` / `EntityCrdt` | typed row CRUD over one root map; text ops | `features/crdt/crdt/src/lib.rs` `LoroRepo`, `EntityCrdt`, `apply_text_ops` |
| Derives | `#[architect(crdt)]` + field `crdt(text)`; `entity_crdt!` | `crates/architect-derive/src/lib.rs`; `features/crdt/crdt-derive/src/lib.rs` `entity_crdt` |
| Codec and text | scalar codecs; `TextOp`; `write_text` (minimal diff), `apply_text_diff`, `upgrade_to_text`, `read_text_or_legacy` | `features/crdt/crdt/src/codec.rs` `write_text`, `upgrade_to_text`, `read_text_or_legacy` |
| Schema versions | stamp + ordered, per-step-committed migrations | `features/crdt/crdt/src/schema.rs` `schema_version`, `set_schema_version`, `migrate`, `Migration`, `SchemaStamp` |
| Persistence | trait + File, Turso, IndexedDB, InMemory, SeaORM | `features/crdt/crdt/src/persistence.rs` `Persistence`; `features/crdt/crdt/src/fs.rs` `FilePersistence`; `features/crdt/crdt-seaorm/src/lib.rs` `SeaOrmPersistence` |
| SeaORM tables | crate `crdt-seaorm` owns `crdt_doc` + `crdt_update` and their `Migrator` | `features/crdt/crdt-seaorm/src/migrations.rs` `Migrator`; `features/crdt/crdt-seaorm/src/entities.rs` |
| Hub sync | `DocSync` call = session; version-vector catch-up; fan-out | `features/crdt/crdt/src/sync.rs` `DocSync`, `DocSyncHost`, `SyncedDoc` (`new`, `durable`, `run`) |
| Presence | `EphemeralStore` presence; sliding fan-out | `features/crdt/crdt/src/sync.rs` `DocPresence`, `PresenceHost`, `PresencePeer`, `PresenceDriver` |
| `DocRegistry` | many docs on one dispatcher pair; admission, authorizer, eviction | `features/crdt/crdt/src/registry.rs` `DocRegistry`, `with_admission`, `with_authorizer`, `with_idle_eviction` |
| `crdt::peer` | pairwise exchange, `DocPeer` lane, `SelfLane`, `pull`, lineage check | `features/crdt/crdt/src/peer.rs` `DocPeer`, `DocPeerHost`, `SelfLane`, `exchange`, `pull`, `is_foreign_lineage` |
| `crdt::docs` | catalog of schema-agnostic app docs (client-chosen id, opaque `kind`, `parent`, tombstones, contiguous `rev`); its host; the device driver | `features/crdt/crdt/src/docs/mod.rs` `Docs`, `DocMeta`, `DocChange`, `DocsStore`, `InMemoryDocsStore`; `features/crdt/crdt/src/docs/host.rs` `DocsHost`; `features/crdt/crdt/src/docs/driver.rs` `DocsDriver`, `DocsIndex`, `DocStorage`; `features/crdt/crdt/src/docs/turso.rs` `TursoDocsIndex`; `features/crdt/crdt-seaorm/src/docs.rs` `SeaOrmDocsStore` |
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
| An app's own data (projects, setlists, rigs) on every device of an owner, made offline, listed without opening | `crdt::docs`: `DocsHost` on the hub, a `DocsDriver` per owner on each device (Recipe D) |
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
- Driver by hand: `SyncedDoc::new(doc_id, doc)` once per replica, before local edits; a device replica adds `.durable()` so what arrives from the hub is persisted too (`apply_remote_durable`); then `run(&client)` per connection, re-run on disconnect.
- Server: `DocRegistry` with `DocSyncDispatcher` and `DocPresenceDispatcher` on one router.
- Server storage on SeaORM: `SeaOrmPersistence::new(db)` over the `crdt_doc` / `crdt_update` tables. Run `crdt_seaorm::Migrator` first: alone (`Migrator::up(&db, None)` via `sea_orm_migration::MigratorTrait`) when CRDT bytes are all the database holds, otherwise composed into the feature's own migrator (`let mut m = crdt_seaorm::Migrator::migrations(); m.extend(ours); m`). Two migrators on one database share `seaql_migrations`, and each refuses the other's applied rows, so compose rather than run both. Its initial migration is named `migrations` (`DeriveMigrationName` takes the file stem); don't give one of yours the same name.
- Test oracles: `replicas_converge_and_late_joiners_catch_up`, `offline_history_pushes_back_after_restart` (`features/crdt/crdt/tests/sync_convergence.rs`); `features/crdt/crdt/tests/registry.rs`.

## Recipe D: app documents (mirrors `features/crdt/crdt/tests/docs.rs`)

The host stores and relays Loro bytes and a catalog row (`DocMeta`); it never reads or writes a doc. Content syncs over `DocSync` under the doc's id; the `Docs` lane carries only the catalog.

- Host: `DocsHost::open(store, registry)`; `store` is `InMemoryDocsStore` (tests) or `crdt_seaorm::SeaOrmDocsStore` (its `crdt_doc_meta` table comes with `crdt_seaorm::Migrator`), `registry` a `DocRegistry` over any `Persistence`. Mount it for `Docs`, its stream and `DocSync` (and `DocPresence`): `.with(docs_rpc_service_descriptor(), serve(host.clone())).merge(stream_layer(host.clone())).with(doc_sync_service_descriptor(), DocSyncDispatcher::new(host.clone()))` (all in `crdt::docs`, the sync ones in `crdt::sync`). A host that also serves other docs routes `DocSync` by `host.admits(doc_id)`.
- Per-caller rules: `.with_authorizer(|who, doc_id, DocAccess::Read | DocAccess::Write| ..)`; default allow, leaving it to the permissions gate. The `changes` hub is not filtered per caller: one host per audience.
- Device: `DocsDriver::new(index, storage)` per owner (personal space, each org), with `TursoDocsIndex` (or `InMemoryDocsIndex`) and a `DocStorage` resolving each doc's `Arc<dyn Persistence>` from its `DocMeta` (`SharedStorage` for one database). Open docs only through `create` / `open`, so each replica is wired for sync before it is edited. `run(&DocsLane { docs, changes, sync })` is one connected session; loop it with backoff.
- What `run` does: subscribe, wait for `DocChange::Attached`, register pending creates (`parent_first`), list from the index's cursor (tombstones included), adopt new rows through `DocStorage` (parent first), then sync every live doc with `SyncedDoc::durable` and follow `changes`. A frame whose `rev` skips one triggers a re-list.
- Raw clients: `DocsClient` / `DocsStreamClient` inherent methods return `VoxError<DocsError>`; fold with `architect::vox_into_app`.

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

- **A `crdt::docs` host must never write into a doc.** A doc made offline meets an empty one on the host; anything the host wrote would make it a different lineage, refused as `FOREIGN_LINEAGE`. Create a doc on one device and let others adopt it; two devices each creating "the same" doc offline are two docs.
- **Loro subscription closures must hold `WeakCrdtDoc`.** A strong `CrdtDoc` dropped inside Loro's unsubscribe path deadlocks.
- **Imports don't fire local-update subscriptions.** `apply_remote` is not persisted; relaying hosts use `apply_remote_durable`. `SyncedDoc` imports with `apply_remote` by default, so a hub client keeps only its own edits and re-learns the rest by version-vector catch-up (fine for a browser tab). A device replica, which must still show a received edit after an offline restart, uses `SyncedDoc::new(..).durable()`.
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
