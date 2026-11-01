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
  `CrdtDoc` and its sync keep their replica in the file.

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
one `NodeAccess` (its `Devices` and its `Shares`) and serves every
connection through `node_router`, which resolves the caller from the
endpoint id the handshake proved:

```rust
let access = NodeAccess::new(Devices::new(), Shares::new());
iroh_link::serve_peers(&endpoint, move |remote| {
    node_router(reg.clone(), OpenDocs(reg.clone()), access.clone(), &remote.to_string())
});
```

- **Share links.** `shares.share(&endpoint.id().to_string(), doc_id)`
  returns a `DocLink`; its text is `fts-doc:<endpoint>/<doc>?key=<key>`.
  Whoever holds it edits that one document until `shares.revoke(&link)`. The
  other side parses it, dials the endpoint, presents the key with
  `LinkKey::new(&link)` as middleware on a `DocSyncClient`, and runs
  `SyncedDoc::new(link.doc_id, doc).run(&client)`. A link is a credential;
  there are no read-only links yet.
- **The person's devices.** Keep `Devices` current from the auth server's
  device registry: `devices.replace(directory.peer_endpoints(this_device))`
  on every `list_devices` or `register_device` answer. A registered device
  may sync any document; a device taken off the account is refused on its
  next call. Registering is in the [identity chapter](@/guide/identity.md).
- **On the LAN.** Bind the endpoint with `PeerLookup::Lan` or
  `PeerLookup::N0AndLan`, advertise with `advertise_name(&endpoint, "Cody's laptop")`,
  and browse with `LanBrowser::new(endpoint.id())?.events()`
  (`LanEvent::Found` / `Lost`); dial a found id bare
  (`crates/architect/src/iroh_link/lan.rs`). An iOS app lists `_fts._udp`
  in `NSBonjourServices`.

`features/crdt/crdt/tests/p2p.rs` is all three over real iroh endpoints.

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
