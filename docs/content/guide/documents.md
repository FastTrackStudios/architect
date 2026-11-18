+++
title = "Documents, sync and files"
description = "An app's work as CRDT documents: kinds and named undo, the document file, sync to the org through the org driver, peer to peer by link and on the LAN, presence and collaborative text, blobs by hash, and Files."
weight = 110
+++

An app's work (a song, a note, a chart, a rig library) is a **document**: a
Loro CRDT that every device edits offline and that merges when they meet.
architect owns how a document is shaped, saved, undone, synced and shared;
the app declares its kinds and edits. In this chapter you will:

1. declare a document kind and edit it through named, undoable steps;
2. keep it in a document file, with checkpoints, branches and crash
   recovery;
3. sync it to the org's server with one driver per org;
4. share it live peer to peer: by link, with the person's own devices, and
   with devices found on the LAN;
5. show who is here and bind a collaborative text field;
6. keep its audio and images as blobs by hash, fetched on demand;
7. store files in an org's roots with `architect-files`.

All of it is the `crdt` crate (`features/crdt/crdt`) unless said otherwise.
Each step names the test that proves it; read the test beside the step.
A crate that depends on `crdt` from outside the fleet's feature union (a
library with no UI) patches architect's workspace-hack out; the recipe is
in `skills/architect/references/xtask-tooling.md` ("Outside the union").

Rows a server owns and validates (a task's status, a member list) are not
documents: they are entities and services with optimistic updates (the
`architect-atom` store). Use a document where concurrent edits must both
survive.

## 1. A kind, and named undo

A `DocKind` is an id, a name, a file extension, a schema stamp with its
migrations, and which blobs it points at (`features/crdt/crdt/src/kinds.rs`):

```rust
use crdt::{DocKind, Migration};

fn song_audio(doc: &LoroDoc) -> Vec<String> { /* the 64-hex hashes the song names */ }

let kind = DocKind::new("studio.song", "Song")
    .extension("session")
    .schema(2, vec![
        Migration { from: 0, name: "seed roots", run: seed_roots },
        Migration { from: 1, name: "notes to text", run: notes_to_text },
    ])
    .blobs(song_audio);
```

Register every kind in a `DocKinds` (`register`, `get`, `for_extension`) so
an opened file finds its kind by extension. Migrations run one stamp at a
time and must be idempotent: two offline devices may both run one, and
both runs merge.

Every change is a step with a name, undone as one (`features/crdt/crdt/src/history.rs`
`History`):

```rust
let mut history = History::new(&doc, 200);               // keep 200 steps
history.transact(&doc, "Arm track", |doc| { /* edit */ });
history.undo();                                          // Some("Arm track")
```

Undo is local: Loro's undo manager undoes only this peer's changes, so
undoing my step never undoes a collaborator's. A step can span many calls:
`begin(name)` opens one, every `transact` until `commit(&doc)` folds into it,
and `abort(&doc)` takes it all back. That is how a script that runs ten
actions undoes as one step (the [extending chapter](@/guide/extending.md)).
Share one `Arc<Mutex<History>>` between everything that edits the document.

## 2. The document file

One file per document (`features/crdt/crdt/src/file/mod.rs` `DocFile`): an
append-only record file with the kind, schema and id in its meta, branches,
named checkpoints, and a blob table. Nothing in it is native, so a browser
reads a file it was handed (`DocFile::from_bytes`) as a desktop reads one
from disk (`DiskStorage`).

```rust
use crdt::file::{DiskStorage, MAIN};
use crdt::DocFile;

let file = DocFile::create(DiskStorage::open(&path)?, &kind).await?;  // or DocFile::open
let mut main = file.open_branch(MAIN).await?;                          // migrated to the kind's schema
main.transact("Split item", |doc| { /* edit */ }).await?;              // on disk when it returns
main.save("Mix sent to Dan").await?;                                   // a named checkpoint, folded
let mut alt = main.fork("acoustic").await?;                            // a branch, open side by side
main.merge(&alt).await?;
let back = file.restore(&file.versions(MAIN).await?[3], "from v3").await?;
```

- **No unsaved state.** `transact`, `undo` and `redo` return once the step
  is on disk; "Save" names a version.
- **Crash.** A write cut short is a torn last record; `open` drops it
  (`DocFile::recovered` says so) and replays the steps since the last
  snapshot. `kill_9_mid_edit_reopens_with_every_finished_step` in
  `features/crdt/crdt/tests/file.rs` kills a writer mid-edit to prove it.
- **Kind.** Opening refuses another kind or a newer schema.
- **Sync.** `file.persistence(MAIN)` is the branch as a `Persistence`, so a
  `CrdtDoc` and its sync keep their replica in the file. Make its
  `SyncedDoc` durable (`SyncedDoc::new(id, doc).durable()`): imports fire
  no local-update subscription, so without it the file keeps only this
  device's edits and what arrived from others is gone after a restart
  (`a_synced_file_replica_keeps_what_it_received_only_when_durable` in
  `features/crdt/crdt/tests/file.rs`). The org driver does this for you.

Not yet: undo that survives reopening, an IndexedDB `Storage`, one writer
per file across processes.

## 3. Sync to the org: the org driver

A signed-in device keeps one driver per org it has documents in
(`features/crdt/crdt/src/docs/org.rs`). `OrgSyncs` makes each org's
`OrgDocs` on first ask and keeps it connected; `Placement` puts each
document in its file. Shortened from `features/crdt/crdt/tests/org_sync.rs`:

```rust
let syncs = OrgSyncs::new(
    move |org: Org| {                           // once per org
        let placement = Placement::new(dir.join(&org.slug), kinds.clone());
        async move { Ok(OrgDocs::new(org, TursoDocsIndex::open_path(&index).await?, placement)) }
    },
    move |org: Org| async move {                // every dial: the token as it is now
        match signed_in() {
            None => Dialed::Stop,
            Some(token) => match lane(&org_url(&org.slug), &token).await {
                Ok(lane) => Dialed::Lane(lane),
                Err(e) => Dialed::Unreachable(e),
            },
        }
    },
);
let docs = syncs.current(&orgs).await?;         // the window's org; None in All
```

- **Staying connected.** Each org dials, runs a session until the
  connection closes (`Caller::closed`), waits under architect's `Backoff`
  (full jitter, 250 ms to 10 s), and dials again; `Dialed::Stop` (signed
  out) ends it. `link()` is `Connecting`, `Live`, `Offline` or `Stopped`,
  for a status item. An app whose connection is already supervised
  (`use_app_supervised`) calls `run_on(&lane)` once per
  `Connection::generation` instead.
- **Placement.** `docs.placement().place(id, path)` before the driver opens
  a document puts it in the file the person chose; otherwise it is
  `<dir>/<id>.<extension>`. A document made on this device gets a stamped
  file; one made elsewhere an empty one (`DocFile::adopt`) that fills when
  it syncs, so it is never another lineage.
- **Following the person.** `syncs.follow(&orgs)` stops the orgs the person
  left; their files stay.
- **Offline.** Every document edits locally; the next session is a delta
  both ways (`a_members_doc_syncs_through_a_dropped_connection_and_merges_offline_edits`).

### On the server: who may read and write

The org's server hosts the catalogue and the content: `DocsHost` over a
`DocRegistry`, both asking the org's permission engine on every call
through `DocPermissions` (`features/crdt/crdt/src/registry.rs`), which
checks `read` or `write` on `org/<org>/doc/<id>`:

```rust
let p = DocPermissions::new(org_engine).in_org(&org.id);
let host = DocsHost::open(store, registry.with_permissions(p.clone())).await?.with_permissions(p);
```

One host per org, mounted on the org's lane behind the permissions gate
(the [identity chapter](@/guide/identity.md)). Every refusal answers
"unknown document", so a caller cannot tell forbidden from absent.

A caller who may `read` a document but not `write` it **follows** it on
the registry: its sync session brings the document and every later edit
down, and the first update it sends up ends the session with
`SyncError::Forbidden`, nothing of it applied. Its presence shows it
everyone (a leader's transport included) and drops what it publishes, and
`DocPresence::now` answers it as anyone. A reader's app runs
`SyncedDoc::new(doc_id, doc).read_only()`, which never sends up. Without
`DocPermissions`, `registry.with_authorizer(..)` says who may open (read)
and `with_write_authorizer(..)` who may also write; the second defaults to
the first (`a_reader_follows_live_and_cannot_write`,
`a_readers_presence_reads_and_publishes_nothing` in
`features/crdt/crdt/tests/registry.rs`).

### Lineage

A document is created once, on one device, and adopted by the others. Two
devices that each create "the same" document offline under one id have
made two documents: the host refuses the second as foreign lineage
(`Lineage::Strict`, the default). A document every device makes offline
under one derived id, merged per key, is `Lineage::Merge` (the person's
synced settings are one). A host never writes into a `Strict` document.

## 4. Peer to peer

Three ways a document reaches another device without the org's server,
all over vox on iroh (`features/crdt/crdt/src/p2p.rs`). Each node keeps
one `NodeAccess` (its `Admission` and its `Shares`) and serves every
connection through `node_router`, which resolves the caller from the
endpoint id the handshake proved:

```rust
let access = NodeAccess::new(Admission::new(), Shares::new());
architect_devices::serve(&endpoint, access.devices.clone(), move |remote| {
    node_router(reg.clone(), OpenDocs(reg.clone()), access.clone(), &remote.to_string())
});
```

- **Share links.** `shares.share(&endpoint.id().to_string(), doc_id)`
  returns a `DocLink`; its text is `fts-doc:<endpoint>/<doc>?key=<key>`.
  Whoever holds it edits that one document until `shares.revoke(&link)`. The
  other side parses it, dials the endpoint, presents the key with
  `LinkKey::new(&link)` as middleware on a `DocSyncClient`, and runs
  `SyncedDoc::new(link.doc_id, doc).run(&client)`. A link is a credential.
  `shares.share_with(&endpoint_id, doc_id, Access::Read)` makes a **read
  link**: its guest follows the document live and writes nothing (an edit
  it sends ends its session), so it joins with `SyncedDoc::read_only()`.
  Its text carries `&access=read`, for display only: the node decides from
  the key. Hand `access.admits` to the registry's `with_authorizer` and
  `access.admits_write` to `with_write_authorizer`; `node_router` holds a
  read link's guest to reading either way. Revoking cuts both kinds
  (`a_read_link_follows_and_cannot_write`).
- **The person's devices.** Run the account pass,
  `architect_devices::account::pass(&devices_client, &token, &this_device, &node, &access.devices)`
  (or `drive`, on every reconnect): it registers this device and lets the
  account's others into the `Admission`. An account device may sync any
  document; a device taken off the account is refused on its next call and
  its connection closes. Registering is in the
  [identity chapter](@/guide/identity.md).
- **A paired device.** A device on nobody's account redeems a pairing offer
  (a six-digit code or a `fts-pair:` ticket) made with
  `NodeAccess::pairing(Pairings::new(...))`, and may sync the documents the
  offer named. See [Devices](@/architecture/devices.md).
- **On the LAN.** Bind the endpoint with `PeerLookup::Lan` or
  `PeerLookup::N0AndLan`, advertise with `advertise_name(&endpoint, "Cody's laptop")`,
  and browse with `LanBrowser::new(endpoint.id())?.events()`
  (`LanEvent::Found` / `Lost`); dial a found id bare
  (`crates/architect/src/iroh_link/lan.rs`). An iOS app lists `_fts._udp`
  in `NSBonjourServices`.

`features/crdt/crdt/tests/p2p.rs` is all three over real iroh endpoints.

**A document file in a vault syncs like a note.** The app's node keeps
each vault note as a document; a vault file of a kind an extension declares
is one too, so a Keyflow `.session` in a vault reaches the person's other
devices, and the org's node, with no sync host of its own. The extension
declares the kind in its manifest (`[[doc_kinds]]` with `files` and an
optional `schema`); the node registers it while the extension is compiled
in, installed, or loaded in the app's window — a development copy
(`architect-desktop dev --extension <folder>`, `EXT_DEV`, `EXT_CLONE`)
among them (`apps/desktop/src/node/kinds.rs` `ExtensionKinds`, into the
shared `features/crdt/crdt/src/kinds.rs` `SharedDocKinds`; the window's
extension host's contributions are followed by `kinds::follow` into
`Parts::loaded_kinds`). It takes the kind off when the extension is
uninstalled, unloaded, or reloaded without it — unless another of those
still declares it. Such a file is listed by the id in its
header (`features/crdt/crdt/src/file/mod.rs` `DocFile::peek`), opened with
the file itself as its storage, its `main` branch, and attached to as a
note is, under the vault's write guard for the file; another device makes
it empty, of the same lineage, before the first session fills it
(`features/vault/vault-collab/src/lib.rs` `VaultCollab::register_document`).
`apps/desktop/tests/node.rs` `an_extensions_document_syncs_between_two_nodes_like_a_note`
is two nodes doing it. An app edits the node's copy
(`apps/desktop/src/node/mod.rs` `Running::document`), not the file: the file
has one writer at a time.

**An extension edits the node's copy.** The extension that owns the kind
never opens the file either. It names the document by its vault path and
syncs a replica of the node's copy through the host, linked in or as a
process (`features/extensions/src/live.rs`):

```rust
// In an action or a view: `handle` is its `linked::Handle`
// (`guest::document(path)` does the same in a plain process guest).
let lane = handle.document("Songs/Opener.session").await?; // LiveDoc: doc_id, kind, writable
let replica = lane.replica();          // an in-memory CrdtDoc, kept in step
replica.attached().await?;
let body = replica.doc().loro().get_text("body");
body.insert(0, "Intro")?;
replica.doc().loro().commit();         // reaches the node, its file, every device
// Or drive a `SyncedDoc` of your own CrdtDoc: `lane.sync(&mut synced).await`.
```

The host answers `live_document` only for a file of a kind the extension's
manifest declares (`[[doc_kinds]]`), and only with `reads = ["documents"]`;
its edits reach the node only with `writes = ["documents"]`, otherwise it
follows (a share-link guest, whose capabilities have no `writes`, only
follows). The session is the ordinary `DocSync` one, on the connection a
process already has to the host (a linked extension's is in memory),
relayed to the node's registry as the extension, for whoever it acts for, so
the registry's authorizer and the vault's write guard decide
(`features/extensions/src/live.rs` `ExtensionDocSync`). The app gives its
host the node's documents with `Host::set_kind_documents`
(`features/extensions/src/live.rs` `RegistryDocuments` over a
`VaultCollab`). The Architect app does it as its node starts
(`apps/desktop/src/node/mod.rs` `Running::kind_documents`, handed to the
window's host by `apps/ui/src/extensions.rs` `on_host`): a path is in the
node's one vault, or, when it holds several, its first folder names the
vault (`apps/desktop/src/node/mod.rs` `vault_path`), and a session attaches
under the vault's write guard. `features/extensions/tests/live_documents.rs`
is a process extension editing a `.session` in a vault and following the
node's edits, a linked one that may only read, and the refusals;
`apps/desktop/tests/node.rs`
`the_windows_extension_edits_the_nodes_copy_of_its_document` is the same
against a running node.

Deleting such a file deletes the document everywhere, as a note's delete
does. A file the node listed and finds gone at its next listing
(`features/vault/vault-collab/src/lib.rs` `VaultCollab::notice_deletes`,
after `apps/desktop/src/node/catalog.rs` `VaultCatalog::notes`) sets the
document's tombstone (`meta.deleted`), which reaches every other holder and
removes its file there. A deleted document is kept under the node's
`crdt/deleted-documents/`, which also says the id is deleted: a peer that
still lists it, a later sweep or a restart never makes its file again
(`features/vault/vault-collab/src/documents.rs`). A file of it put back (a
restore) is the document again on its next open. One deleted while nobody
had it open is tombstoned as soon as a peer's history reaches it.
`apps/desktop/tests/node.rs` `an_extensions_document_deleted_on_one_node_is_deleted_on_the_other`
is two nodes doing it.

One document is one file per node. A copy carries the id it was copied
from: while the first file is there the copy is not listed or synced (and
is said once in the log); give it an id of its own to keep both. A rename
moves the document with the file on that node, with no delete and no
second file, but is not sent: another node keeps its file at the path it
has. A delete the node did not see happen (the file removed while the node
was stopped) is not noticed yet.

<!-- media: doc-share-link-join — a share link copied on one device and opened on another, the document joining live -->
<!-- media: lan-device-found — a device on the same network appearing by name, ready to sync -->

## 5. Presence and collaborative text

### Who is here

Presence rides beside each document and is never in it
(`features/crdt/crdt/src/presence.rs`). Publish yours with `Here` (your
state on change, your pointer up to 30 times a second, your playhead) and
read everyone's with `Roster`. In Dioxus, `use_here`, `use_roster` and
`use_presence_frame` (`features/crdt/crdt/src/hooks.rs`).

A pointer is a **place**, not pixels: a space and named coordinates, so it
lands on the same bar of the same track however differently two people
have scrolled:

```rust
here.pointer(Place::new("session.arrange").with("time", t).with("track", id), clock_ms());
// `arrange_pixels` is the view's: a place in its pixels, or None when it does not show it.
let marks = roster.pointers(clock_ms(), |place| arrange_pixels(place));
```

Draw them with the workspace's `PresenceLayer` as the view's last child
(the [UI chapter](@/guide/ui.md)). Never call `sweep()` in a render.

### A playhead, in the host's clock

A playhead in presence (`PlayState`) says where it was and *when*, so a
peer extrapolates it every frame instead of streaming it. "When" has to be
one clock for both peers, and two processes never share one: each
`clock_ms` starts at its own first call. So the presence host serves its
clock (`DocPresence::now`), each `PresenceDriver` pings it ten times a
second into its peer's `SharedClock` (`features/crdt/crdt/src/clock.rs`,
NTP's estimate over the fastest quarter of the last 32 exchanges), and a
playhead is stamped in it:

```rust
// The leader: false until the driver has an estimate.
here.play_now(playing, position_seconds, rate);
// A follower, each frame:
if let (Some(play), Some(now)) = (play_state, peer.clock().now_ms()) {
    let at = play.position_at(now);
}
```

The error is at most half the fastest round trip (`SharedClock::round_trip_ms`):
under a millisecond on a LAN. A host older than this serves no clock; the
driver's pings then stop and presence carries on, with no estimate.

<!-- media: presence-pointers-overlay — two people in one arrangement: the other's named pointer and selection wash drawn over the pane -->

### Collaborative text

A `LoroText` bound to an editor (`features/crdt/crdt/src/text.rs`
`TextBinding`): the editor reports its edits as operations, others' edits
come back echo-guarded and transformed against what the person typed, and
carets are stable cursors.

```rust
// `editor` is your editor, implementing `EditorBinding`.
let binding = TextBinding::new(&doc, doc.get_text("body"));
editor.set_text(&binding.shown());           // seed it
binding.on_local(&ops)?;                     // each edit, in the editor's coordinates
binding.sync_into(&mut editor);              // after every document revision
let caret = binding.caret(anchor, head);     // for PeerState::caret
let others = binding.remote_carets(&roster);
```

`set_offset` hides a prefix the editor does not show (a note's
frontmatter); `set_composing` brackets IME composition. Positions are
Unicode scalars, so an editor in UTF-16 translates at its edge.

<!-- media: collab-text-two-carets — two people typing into one note at once, each other's named caret and selection shown -->

## 6. Blobs by hash

Audio and images never go in the Loro document: the document holds their
sha256, and the kind's `blobs` function reads the hashes out. The bytes
live in a `LocalFs` store and travel through `architect::blob`'s `Fetcher`
(`crates/architect/src/blob/fetch.rs`):

```rust
use architect::blob::{Fetcher, HttpRemote, LocalFs, VoxRemote};

let fetcher = Fetcher::new(LocalFs::new(data_dir.join("blobs")))
    .with_remote(Arc::new(VoxRemote::new(peer_client)))       // the peer the song came from
    .with_remote(Arc::new(HttpRemote::new(server_blobs)));    // the org server's copy
let reader = fetcher.open(hash).await?;                        // streams as it arrives
```

- One transfer runs per hash however many readers open it; a dropped link
  keeps the partial file and the next fetch resumes (`set_remotes` after a
  reconnect). The whole blob is verified before it is kept.
- A node serves its store with `BlobTransferHost` on the same router as
  `DocSync`; `.allow(|hash| …)` narrows what it sends. A server's HTTP route
  answers ranged requests with `serve_http`.
- **Pinning.** `crdt::blobs::keep_offline(kind, doc, &fetcher)` pins and
  fetches every blob a document points at; `release` unpins them.
- **Collection.** `store.collect(&crdt::blobs::live(&kinds, docs), grace)`
  removes every blob no document on the device points at and nothing pins.

`features/crdt/crdt/tests/blobs.rs` records a take on one device and plays
it on another.

## 7. Files: `architect-files`

Files that are not a document's (an org's shared folders, impulse
responses, sample packs) go in `architect-files` (`features/files`): roots
of files and folders, a version per save, labels, resumable and
deduplicated uploads over vox channels, a quota, and access by role plus
per-folder grants. Its bytes are the same `architect::blob` store, so a
song's audio can be both a file in a root and a blob its document points
at.

The app's side:

```rust
let files = FilesClient::new(caller);   // the org lane's vox Caller
let irs = files.ensure_root("signal/impulse-responses", "Impulse responses").await?;
let saved = files.put(irs.id, "Cab/4x12.wav", &wav, Save::create_only()).await?;
// next time: Save::replacing(saved.content), refused as Stale if it moved on
```

The server mounts `FilesServer` on each org's lane behind the gate, with
`ConfinedEngine` so only members reach it; the org comes from the lane,
never from an argument. `skills/architect/references/files.md` has the
mount, and `features/files/tests/files.rs` the whole loop.

## Where Task and Studio are

Architect-side today: kinds, named undo, the file, the org driver,
presence, the text binding, blob transfer and the Files core. Still the
apps' to adopt: Studio's `session-format` moving onto `DocFile`; Task's and
Studio's org drivers moving onto `OrgSyncs`; `editor-crdt` implementing
`EditorBinding` for Task's notes and Studio's chart editor; Studio's
arrangement publishing presence places. Task and Studio still run Task's
own Files; client sync, the browser and desktop integrations and the HTTP
byte face of `architect-files` are not built yet.

## Where to read more

- [Documents](@/architecture/documents.md): the layer, the file format,
  the org driver, Files, text, presence.
- [Collaboration](@/architecture/collaboration.md): `CrdtDoc`, typed
  repositories, hooks, `DocRegistry`, schema migrations.
- [Peers](@/architecture/peers.md): pairwise exchange, devices and links.
- [Blobs](@/architecture/blob.md): `Fetcher`, serving, pins and collection.
- `skills/architect/references/crdt-peer-sync.md` and
  `skills/architect/references/files.md`.
