# crdt: documents, text, schema, hub sync, `crdt::peer`, `crdt::p2p`, DocRegistry

Crate `crdt` (`features/crdt/crdt`); guides `docs/content/architecture/collaboration.md` and `docs/content/architecture/peers.md`. Feature gates: `vox` (sync, registry, peer), `dioxus` (hooks), `turso`, `indexeddb`, `blob` (a document's blobs, native).

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
| Presence in views | per person `<id>/state`, `<id>/pointer`, `<id>/play`; a pointer is a `Place` in a view's coordinate space (never pixels); smooth remote pointers | `features/crdt/crdt/src/presence.rs` `Here` (`edit_state`, `pointer`, `no_pointer`, `play`, `flush`, `announce`), `Roster` (`sync`, `here`, `pointers`, `selections_in`, `carets_in`, `moving`), `Place`, `PeerState`, `Selection`, `TextCaret`, `PointerTrail`, `PresenceSink`, `clock_ms` |
| Collaborative text | a `LoroText` bound to an editor: local edits as ops, others' back echo-guarded and transformed, carets as stable cursors, a hidden prefix, IME | `features/crdt/crdt/src/text.rs` `TextBinding` (`on_local`, `pull`, `sync_into`, `caret`, `resolve`, `remote_carets`, `set_offset`, `set_composing`), `EditorBinding`, `RemoteCaret` |
| `DocRegistry` | many docs on one dispatcher pair; admission, the org's permissions, authorizer, eviction | `features/crdt/crdt/src/registry.rs` `DocRegistry`, `with_admission`, `with_permissions` (`DocPermissions`, `DocAccess`), `with_authorizer`, `with_idle_eviction` |
| `crdt::peer` | pairwise exchange, `DocPeer` lane, `SelfLane`, `pull`, lineage check | `features/crdt/crdt/src/peer.rs` `DocPeer`, `DocPeerHost`, `SelfLane`, `exchange`, `pull`, `is_foreign_lineage` |
| `crdt::link` | a link to a live document, `fts-doc:<endpoint>/<doc>?key=<key>` (wasm-clean); `LinkKey` presents its key on a `DocSyncClient` | `features/crdt/crdt/src/link.rs` `DocLink`, `LinkKey`, `LinkError` |
| `crdt::p2p` | who a node serves, peer to peer: the owner's devices (from auth's device directory) and live links; the router one connection gets | `features/crdt/crdt/src/p2p.rs` `Devices`, `Shares`, `NodeAccess`, `node_router` |
| `crdt::docs` | catalog of schema-agnostic app docs (client-chosen id, opaque `kind`, `parent`, tombstones, contiguous `rev`); its host; the device driver | `features/crdt/crdt/src/docs/mod.rs` `Docs`, `DocMeta`, `DocChange`, `DocsStore`, `InMemoryDocsStore`; `features/crdt/crdt/src/docs/host.rs` `DocsHost`; `features/crdt/crdt/src/docs/driver.rs` `DocsDriver`, `DocsIndex`, `DocStorage`, `DocSyncEvent`, `Refusal`; `features/crdt/crdt/src/docs/device_host.rs` `DriverHost`; `features/crdt/crdt/src/docs/turso.rs` `TursoDocsIndex`; `features/crdt/crdt-seaorm/src/docs.rs` `SeaOrmDocsStore` |
| Hooks | Dioxus bridge: synced doc, crdt list/entry, presence | `features/crdt/crdt/src/hooks.rs` `use_synced_doc_keyed_with`, `use_crdt_list`, `use_crdt_entry`, `use_roster`, `use_here`, `use_presence_frame` |
| `crdt::blobs` | the blobs a document points at (its kind's `DocKind::blobs`): live set for collection, keep offline | `features/crdt/crdt/src/blobs.rs` `referenced`, `live`, `missing`, `keep_offline`, `release`; transfer in `architect::blob` (`Fetcher`, `BlobTransferHost`, `VoxRemote`, `HttpRemote`) |
| The org driver | one driver per org on a device, kept connected (dial, session until `Caller::closed`, `Backoff`, dial again); docs placed in document files | `features/crdt/crdt/src/docs/org.rs` `OrgDocs` (`stay_connected`, `run_on`, `link`), `OrgSyncs` (`ensure`, `current`, `follow`, `stop`), `Dialed`, `OrgLink`; `features/crdt/crdt/src/docs/placement.rs` `Placement` |
| iroh seam | peer lookup modes; a handler per connection from the proved endpoint id; LAN browsing | `crates/architect/src/iroh_link.rs` `PeerLookup`, `serve_peers`; `crates/architect/src/iroh_link/lan.rs` `LanBrowser`, `advertise_name`, `LAN_SERVICE`; `features/permissions/src/lib.rs` `Principal` |

## Which model for which datum

| Datum | Model |
|---|---|
| Multi-writer doc; concurrent edits must both survive; one doc per collaboration boundary | `CrdtDoc` + `LoroRepo<E>` |
| Prose or any long text edited concurrently | `#[architect(crdt(text))]` on the `String` field (a `LoroText` child written by minimal diff), or an explicit `LoroText` via `apply_text_ops` |
| Short scalar fields (title, done, id refs) | plain `#[architect(crdt)]` / `entity_crdt!` fields: whole-value LWW per field |
| Who is here, cursors, selections | presence (`EphemeralStore`): never in the doc; expires. Publish with `Here`, read with `Roster`; pointers are `Place`s in a view's space |
| Text two people type into at once (a note, lyrics, a chart) | a `LoroText` + `TextBinding`; the caret goes in `PeerState::caret` |
| Devices share docs with no hub | `crdt::peer` over `DocRegistry` plus Host admission; served with `crdt::p2p::node_router` (Recipe E) |
| Someone across the room joins one document live | a `crdt::link::DocLink` from `crdt::p2p::Shares::share` (Recipe E) |
| A canonical server exists and clients attach | `DocSyncHost` / `DocRegistry` + `SyncedDoc` (hub) |
| An app's own data (projects, setlists, rigs) on every device of an owner, made offline, listed without opening | `crdt::docs`: `DocsHost` on the hub, a `DocsDriver` per owner on each device (Recipe D) |
| The same, per org, signed in, each doc a file the person opens | `OrgSyncs` on the device (one `OrgDocs` per org, `Placement`), `DocsHost::with_permissions` on the org's server (Recipe F) |
| Server-owned rows the server validates | not CRDT: optimistic `Store` (`references/offline-atom.md`) |

## Recipe A: pairwise, no hub (mirrors `features/crdt/crdt/tests/peer.rs`)

1. Node registry: `DocRegistry::new(factory).with_authorizer(|who, doc_id| ..)`. The factory is `FilePersistence` (or `CrdtDoc::ephemeral()` in tests).
2. Serve: `DocPeerHost::new(reg.clone(), OpenDocs(reg.clone()), |endpoint| admitted.contains(endpoint))`; mount `LayerRouter::new().with(doc_peer_service_descriptor(), DocPeerDispatcher::new(host))`, wrapped by `PermissionsGate::new(..).permit(doc_peer_service_descriptor(), &PEER_PERMITS).wrap(router)`. `PEER_PERMITS` (the test's table): `list` read, `sync` write, both on `docs/**`.
3. Identity: the gate's resolver must yield `Principal::Host { endpoint }` from the proved iroh connection, never from metadata. `iroh_link::serve_peers(&endpoint, |remote| router_for(remote))` hands each connection's proved id to the function that builds its gated router; `crdt::p2p::node_router` is that router, ready-made (Recipe E).
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
- Per-caller rules: `.with_permissions(DocPermissions::new(org_engine).in_org(&org.id))` asks the org's engine on every call (`read`/`write` on `org/<org>/doc/<id>`); put it on the registry too, so the content lane checks it. `.with_authorizer(|who, doc_id, DocAccess::Read | DocAccess::Write| ..)` is the closure form; default allow, leaving it to the permissions gate. The `changes` hub is not filtered per caller: one host per audience.
- Device: `DocsDriver::new(index, storage)` per owner (personal space, each org), with `TursoDocsIndex` (or `InMemoryDocsIndex`) and a `DocStorage` resolving each doc's `Arc<dyn Persistence>` from its `DocMeta` (`SharedStorage` for one database). Open docs only through `create` / `open`, so each replica is wired for sync before it is edited. `run(&DocsLane { docs, changes, sync })` is one connected session; loop it with backoff.
- What `run` does: subscribe, wait for `DocChange::Attached`, register pending creates (`parent_first`), list from the index's cursor (tombstones included), adopt new rows through `DocStorage` (parent first), then sync every live doc with `SyncedDoc::durable` and follow `changes`. A frame whose `rev` skips one triggers a re-list.
- Lineage (`CreateDoc.lineage`, set by the first registration; a different one later is `DocsError::LineageMismatch`):
  - `Lineage::Strict` (default) refuses a replica with no shared history (`FOREIGN_LINEAGE`). Use it for docs made once and adopted elsewhere, for anything forked, and for text that would duplicate.
  - `Lineage::Merge` imports it, and Loro joins the two histories. Use it for a doc every device makes offline under one derived id (UUID v5), whose content is root containers by name (one root map keyed by entry), so two copies merge per key or field.
- Sync state: `driver.sync_events()` sends `DocSyncEvent`:
  - `Attached { doc_id, lane }` once a session has applied the host's backlog and pushed what the host lacked (gate first-run seeding on it);
  - `Refused { reason: Refusal::ForeignLineage | NotServed | Other }`;
  - `Detached`;
  - `Reset`.

  `driver.reset(doc_id)` drops a `Strict` doc's local copy (`DocStorage::discard`; the default compacts it to an empty snapshot) and takes the hosts'. Open the doc again after `Reset`.
- Peer to peer, with devices as hosts:
  - `driver.host(|who| ..).await` is a `DriverHost`, the same `Docs` + stream + `DocSync` traits backed by the driver's index and open docs, so there is no second writer. Mount it like `DocsHost` on the device's iroh router behind a gate that yields `Principal::Host`. `admit` runs on every call.
  - Another device runs `driver.run_peer(key, &lane)` against it. Sessions run concurrently, one per lane key; `run` is the key `HUB`.
  - Each doc has a relay: an update imported on one lane goes to every other lane and to the device's host, never back to its source.
  - A peer's catalog rows merge in: a tombstone wins and the later rename wins. The hub's `rev` is never overwritten; the peer lane's `rev` is the index's `LocalDoc::seq`.
  - A doc learned from a peer stays pending until the hub session registers it. Rename and delete go through the hub.
- Raw clients: `DocsClient` / `DocsStreamClient` inherent methods return `VoxError<DocsError>`; fold with `architect::vox_into_app`.

## Recipe E: live peer to peer, links and devices (mirrors `features/crdt/crdt/tests/p2p.rs`)

1. One `NodeAccess::new(Devices::new(), Shares::new())` per node. Keep `Devices` current from auth: `devices.replace(directory.peer_endpoints(this_device))` on every `list_devices` / `register_device` answer (`features/auth/auth-proto/src/device.rs` `DeviceDirectory::peer_endpoints`). It is read per call, so a device taken off the account is refused on its next call, over a connection it already has.
2. Registry: `.with_authorizer(move |who, id| access.admits(who, id) || mine(who, id))`. `admits`: a registered device (`Principal::Host`) any document, a link's guest (`Principal::Guest`) only the link's document, anyone else nothing.
3. Serve: `iroh_link::serve_peers(&endpoint, move |remote| node_router(reg.clone(), OpenDocs(reg.clone()), access.clone(), &remote.to_string()))`. Each connection gets `DocSync` + `DocPeer` behind a gate whose engine is the `NodeAccess` (an unregistered endpoint is refused before any service runs) and whose resolver is the proved id, or the link's guest when the call presents a live key.
4. Share: `shares.share(&endpoint.id().to_string(), doc_id)` returns a `DocLink`; hand over `link.to_string()`. `shares.revoke(&link)` / `revoke_doc(doc_id)` refuse it from the next call.
5. Join: `let link: DocLink = text.parse()?`; dial `link.endpoint.parse::<iroh::EndpointId>()?` with `iroh_link::connect`, establish a `DocSyncClient`, `.with_middleware(LinkKey::new(&link))`, then `SyncedDoc::new(link.doc_id, doc).run(&client)`.
6. Devices live: the same `SyncedDoc` over a `DocSyncClient` to the other device, with no `LinkKey` (the device is its endpoint). Documents not open: Recipe A's `pull` over a `DocPeerClient` on the same connection kind.
7. Find devices nearby (`iroh-lan`, native): bind with `PeerLookup::Lan` / `N0AndLan`, `advertise_name(&endpoint, "Cody's laptop")`; `LanBrowser::new(endpoint.id())?.events().await` yields `LanEvent::Found { endpoint, name }` / `Lost`. Dial a found id bare: its address comes from the LAN.

## Recipe F: a document's blobs (mirrors `features/crdt/crdt/tests/blobs.rs`)
A document points at its audio and images by sha256; the bytes never go in the Loro doc.
- Kind: `DocKind::new(..).blobs(song_audio)`, where `song_audio(&LoroDoc) -> Vec<String>` returns 64-hex hashes.
- Recording device: `LocalFs::put_bytes` (or `put`) the take, write its hash into the doc, and serve the store: `.with(blob_transfer_service_descriptor(), BlobTransferDispatcher::new(BlobTransferHost::new(Arc::new(store))))` on the same router as `DocPeer`/`DocSync` (vox over iroh or WebSocket). `.allow(|hash| ..)` narrows what it sends.
- Other device: `Fetcher::new(LocalFs::new(dir)).with_remote(Arc::new(VoxRemote::new(client)))`, then `.with_remote(Arc::new(HttpRemote::new(".../blobs")))` for the server's copy (served with `blob::serve_http`). `fetcher.open(hash)` / `open_at(hash, offset)` streams while it arrives; `fetch(hash)` waits for it whole. One transfer per hash; a dropped link keeps the `.part`, and the next fetch resumes (`set_remotes` after a reconnect).
- Offline: `crdt::blobs::keep_offline(kind, doc, &fetcher)` pins and fetches; `release` unpins.
- Collection: `store.collect(&crdt::blobs::live(&kinds, docs), grace)` with every document on the device; pins are kept, anything else not in the set goes.
- Streamed bytes are verified only at the end: a corrupt transfer that restarts ends the stream with `InvalidData`.
## Recipe F: one driver per org (mirrors `features/crdt/crdt/tests/org_sync.rs`)
```rust
let syncs = OrgSyncs::new(
    move |org: Org| {                       // made once per org
        let placement = Placement::new(dir.join(&org.slug), kinds.clone());
        async move { Ok(OrgDocs::new(org, TursoDocsIndex::open_path(&index).await?, placement)) }
    },
    move |org: Org| async move {            // every try: the token as it is now
        match signed_in() {
            None => Dialed::Stop,
            Some(token) => match lane(&org_url(&org.slug), &token).await {
                Ok(lane) => Dialed::Lane(lane),
                Err(e) => Dialed::Unreachable(e),
            },
        }
    },
);
let docs = syncs.current(&orgs).await?;    // the window's org; None in All
docs.placement().place(id, project_file);  // before the driver opens it
let doc = docs.driver().create(req).await?;
```
- `OrgSyncs::ensure` spawns `stay_connected`: dial, `run_on` until a lane connection's `Caller::closed`, sleep `Backoff::next_delay` (reset on connect), dial again. `link()` / `watch_link()` is `Connecting | Live | Offline | Stopped`. `follow(&orgs)` stops orgs the person left; their files stay.
- An app with a supervised connection (`use_app_supervised`) calls `run_on(&lane)` once per `Connection::generation` instead of `stay_connected`.
- `Placement`: catalogue `kind` = the `DocKind` id; a doc not placed is `<dir>/<id>.<extension>`; content syncs through `DocFile::persistence(MAIN)`; `placement.file(id)` is the open `DocFile` (name steps through its `Branch`). New files: `DocFile::create_as` for a doc made here, `DocFile::adopt` (empty, unstamped) for one made elsewhere.
- Server: `DocsHost::open(store, registry.with_permissions(p.clone())).await?.with_permissions(p)`, one host per org, mounted on the org's lane behind the gate.
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

## Recipe E: a document's file (mirrors `features/crdt/crdt/tests/file.rs`)

```rust
let kind = DocKind::new("studio.song", "Song").extension("session").schema(2, MIGRATIONS);
let file = DocFile::create(DiskStorage::open(&path)?, &kind).await?; // or DocFile::open
let mut main = file.open_branch(MAIN).await?;           // migrated to the kind's schema
main.transact("Split item", |doc| { /* edit */ }).await?; // durable when it returns
main.save("Mix sent to Dan").await?;                     // names a checkpoint, folds
```

- No "unsaved" state: `transact`, `undo` and `redo` return after the step is synced. Edits made straight on `branch.loro()` are written only by the next step or `flush(label)`.
- A crash leaves a torn last record; `open` drops it (`recovered()`) and replays the steps since the last snapshot.
- Branches: `fork`, `open_branch` (side by side), `merge`, `DocFile::delete_branch` (not `main`); versions: `DocFile::versions`, `open_version` (read-only), `restore` (a new branch).
- In the browser: `DocFile::from_bytes(bytes, &kind)`; the whole folded file to send is `to_bytes()`.
- To sync a branch with `DocSync`, back the `CrdtDoc` with `file.persistence(MAIN)`. A file for a catalogue's doc: `DocFile::create_as(storage, &kind, id)` (made here) or `DocFile::adopt(storage, &kind, id)` (made elsewhere, `main` empty).

## Pitfalls

- **A `crdt::docs` host must never write into a `Strict` doc.** A doc made offline meets an empty one on the host; anything the host wrote would make it a different lineage, refused as `FOREIGN_LINEAGE`. Create a `Strict` doc on one device and let others adopt it: two devices each creating "the same" `Strict` doc offline are two docs, and the second is refused until `reset`. A doc every device makes under one derived id must be `Lineage::Merge`.
- **Loro subscription closures must hold `WeakCrdtDoc`.** A strong `CrdtDoc` dropped inside Loro's unsubscribe path deadlocks.
- **Imports don't fire local-update subscriptions.** `apply_remote` is not persisted; relaying hosts use `apply_remote_durable`. `SyncedDoc` imports with `apply_remote` by default, so a hub client keeps only its own edits and re-learns the rest by version-vector catch-up (fine for a browser tab). A device replica, which must still show a received edit after an offline restart, uses `SyncedDoc::new(..).durable()`.
- **Status flashes Live before attach succeeds** in the hook's drive loop; a failed attach then sets `Offline`.
- **Admission sees only the doc id; the permissions and the authorizer see the caller** (`permissions_gate::caller()`). `None` means no identity: refuse (`DocPermissions` asks about it as `Principal::Anonymous`). Every refusal answers `SyncError::UnknownDoc`, so a peer cannot tell forbidden from absent (by design). `DocRegistry::doc()` skips both (server-side reads).
- **A file made for a doc from elsewhere must stay empty until it syncs.** `DocFile::create` stamps `main`, which is history of the device's own: a `Strict` host refuses it as `FOREIGN_LINEAGE`. `Placement` uses `DocFile::adopt` for those.
- **`DocPeerHost` admits only `Principal::Host`.** Users and services are refused on `list` and `sync`.
- **Foreign lineage is refused, never merged.** `pull` counts it in `foreign`; someone must pick one copy. A timed-out exchange is `failed`, not `foreign`; retry next sweep. Only a non-empty digest equal to ours is skipped.
- **Permission denials are refusals, not drops** (`InvalidPayload("permission denied: ...")` → `Refused`).
- **Eviction breaks held handles:** writes reach persistence but not live replicas. Enable `with_idle_eviction` only if handles are re-fetched per use.
- **A pointer is a place, not pixels.** Publish `Place::new("session.arrange").with("time", t).with("track", id)`; draw with `Roster::pointers(now, place)` where `place` puts a `Place` in this view's pixels and answers `None` for a space the view does not show. Draw it with `architect_workspace::PresenceLayer` (`use_presence_overlay`, the view's last child).
- **Text binding:** seed the editor with `TextBinding::shown()`, report its edits with `on_local` (editor coordinates, prefix included), and after every doc revision `sync_into` the editor; its commits carry its own origin and never come back. Ops are Unicode scalars: an editor in bytes or UTF-16 translates at its edge.
- **Presence:** never call `sweep()` in a render (it fires subscribers → render loop). Presence fan-out is sliding(64) and drops; CRDT updates use unbounded `PubSub`, because a dropped update is a lost edit.
- **Persistence:** compaction is not atomic by default (snapshot, then prune); `FilePersistence` appends with plain writes, so a crash mid-write can leave a truncated `.loro`; SeaORM compacts in one transaction and sequences by wall-clock nanos; Turso sequences `MAX(seq)+1` under one connection lock.
- **Text positions are Unicode scalars, not UTF-16:** translate at the DOM boundary. `apply_text_diff` is one prefix/suffix edit. Create the text container once, then sync before forking edits. `read_text_with_migration` uses a mergeable container: prefer `upgrade_to_text` for concurrent upgrades.
- **`#[architect(crdt)]` needs `repo` and a `Uuid` pk.** Without `crdt(text)` a `String` is whole-string LWW.
- **A link is a credential.** Whoever holds the text edits the document until it is revoked; there are no read-only links yet. A link's guest never reaches `DocPeer` (`DocPeerHost` serves devices only).
- **wasm:** registry, peer, `p2p` and the sync host are native only; `crdt::link` parses everywhere; client `SyncedDoc` and hooks compile on wasm; IndexedDB is wasm + `indexeddb`; Turso is native + `turso`; `Persistence` is `?Send` on wasm.
- **Hub and pairwise are different topologies.** A node that is both uses one registry for both. There is no gossip: pairwise sweeps only.

## iroh seam

- `PeerLookup`: `N0` (default), `N0AndLan`, `Lan`; the last two need `iroh-lan` (native only). `Lan` turns relays off and publishes nothing: a LAN-only node cannot reach internet peers. Both LAN modes advertise under `LAN_SERVICE` (`fts`, records `<id>._fts._udp.local.`): an iOS app lists `_fts._udp` in `NSBonjourServices`.
- `serve_router` serves one handler to every caller: only for routers that treat callers alike. Anything gated per peer goes through `serve_peers`, which builds the handler from the endpoint id the handshake proved.
- `bind_endpoint` is `N0`; `bind_endpoint_with` takes a lookup; `EngineHost::iroh_lookup` sets it. Dial by bare id with `iroh_link::connect(endpoint, remote)`. It redials a restarted peer once and reports `DialErrorKind::PeerUnreachable` when that fails too (`crates/architect/src/iroh_link.rs` `connect_with`).
- Test: `crates/architect/tests/iroh_lan.rs` and `devices_on_one_network_find_each_other` in `features/crdt/crdt/tests/p2p.rs` (both need a multicast interface); manual two-machine check `crates/architect/examples/iroh_lan.rs`.
