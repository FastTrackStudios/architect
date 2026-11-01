+++
title = "Documents: one layer for files, undo, sync and peers"
description = "What an app's work is saved as, how it is undone, versioned, synced to a server and to peers, and how its media travels — architect's, so an app only declares its schema."
weight = 57
+++

> **Status: kinds, named undo and the file landed (2026-11-01)** in
> `crdt`: `features/crdt/crdt/src/kinds.rs` `DocKind` (an id, a name, a
> file extension, a schema stamp and migrations; `stamp`, `upgrade`),
> `DocKinds`; `features/crdt/crdt/src/history.rs` `History`
> (`transact` — one named step — `begin`/`commit`/`abort` for a step
> that spans many transactions, `undo`, `redo`, local only);
> `features/crdt/crdt/src/file/mod.rs` `DocFile` and
> `features/crdt/crdt/src/file/branch.rs` `Branch` (the file, below).
> The org driver landed (2026-11-01):
> `features/crdt/crdt/src/docs/org.rs` `OrgDocs` (one org's driver,
> kept connected under a `Backoff`) and `OrgSyncs` (one per org,
> following the person's current org);
> `features/crdt/crdt/src/docs/placement.rs` `Placement` (each document
> in its file); `features/crdt/crdt/src/registry.rs` `DocPermissions`
> (the org's permission on every call).
> **Blob transfer landed:** `DocKind::blobs` (which blobs a document
> points at), `features/crdt/crdt/src/blobs.rs` (`live`, `keep_offline`)
> and `architect::blob`'s `Fetcher` (on demand by hash over vox, iroh or
> HTTP, resumable, streamed), pins and collection ([blobs](@/architecture/blob.md)).
> Studio's
> `session-format` moving onto the file is Studio's work. Proposed
> (2026-10-31): most of
> the pieces exist ([collaboration](@/architecture/collaboration.md),
> [peers](@/architecture/peers.md), [blobs](@/architecture/blob.md)); the
> rest are in Studio and move here. Studio's `studio:docs/design/sync.md` is the
> plan this finishes.

An app's work — a song, a rig library, a note, a chart — is a
**document**. How a document is shaped, written to disk, undone, kept in
versions, synced to the person's other devices and to their org, shared
live with someone across the room, and how the audio and images it
points at travel with it, is one design: every app needs all of it, and
an app that builds any of it alone builds it differently. architect owns
it; an app declares its kinds of document (a schema and its migrations)
and nothing else.

## What exists

In architect (`features/crdt`, `crates/architect`):

- `CrdtDoc`: a Loro document per collaboration boundary, with typed
  repositories (`entity_crdt!`), schema stamps and migrations;
  persistence backends (memory, files, Turso, IndexedDB, SeaORM); the
  `DocSync` and `DocPresence` services; `DocRegistry` (a factory per
  document, admission, idle eviction); the documents catalogue (`Docs`:
  create, list, rename, delete, changes) and the device-side driver;
  peer-to-peer exchange over iroh (`DocPeer`); Dioxus hooks
  (`use_synced_doc`, `use_crdt_list`, `SyncStatus`); vox over iroh; a
  local blob store with resumable fetches.

In Studio, generic and moving here:

- **The file:** a project is one file (`Song.session`): a Loro document
  in a Turso store with `meta`, `blobs` (content-addressed, zstd),
  `branches` (alternatives and checkpoints forked from `main`),
  `updates` (one per undo step) and `snapshots` (folded by compaction),
  readable from bytes in a browser (`session-format`).
- **The codec** that reconciles a serde value into Loro with keyed lists
  (`signal/proto/src/doc/codec.rs`), better than the hand-written one
  here.
- **The org driver:** one sync driver per org, reconnecting, placing
  each document by policy (`studio:features/account/src/sync.rs`) —
  now architect's (below, "The org driver"); Studio moving onto it is
  Studio's work.
- **Live sessions:** a host serves its documents over iroh; a link
  (`fts-session:<endpoint>/<set>`) joins it; presence (pointers, who is
  here) rides beside the document.
- **Named undo** over Loro's `UndoManager`, its steps named by what made
  them.

Stays Studio's: the schemas (the session, Signal's documents, rig books,
frame scenes), the engine's bridge, and REAPER and DAWproject interop.

## The layer

### A document kind

<!-- doc-check: off (proposed names) -->
```rust
Documents::register(
    DocKind::new("studio.song")
        .schema::<SongDoc>()                 // its typed shape (the derive emits the Loro codec)
        .migrations(SONG_MIGRATIONS)         // schema stamp → the next
        .file("session", "Song")             // its file extension and default name
        .blobs(|doc| doc.audio_refs())       // which blobs it points at (they travel with it)
        .undo(Undo::named(200)),             // named steps, kept on disk
);
```
<!-- doc-check: on -->

### The file

One file per document, the format Studio's `.session` already is,
generalised: Loro snapshots and updates, branches, a content-addressed
blob table, a `meta` table with the kind, the schema stamp and the
document's id. An app picks its extension; architect reads and writes
it, folds it on save, and recovers it after a crash from the updates
since the last snapshot. Every finished undo step is on disk the moment
it finishes: there is no "unsaved" state to lose, and "Save" means
"name this version".

Built (`features/crdt/crdt/src/file/mod.rs`): Studio's five tables are
records in one append-only file rather than tables in a database
(`features/crdt/crdt/src/file/format.rs`): a 12-byte header, then each
record framed by its length and a CRC-32. Nothing in it is native, so a
browser reads a file it was handed (`DocFile::from_bytes`) as a desktop
reads one from disk (`DiskStorage`); where the bytes live is a `Storage`
(`MemoryStorage` and `DiskStorage` today).

```rust,ignore
let file = DocFile::create(DiskStorage::open(path)?, &kind).await?; // meta + `main`, stamped
let mut main = file.open_branch(MAIN).await?;
main.transact("Split item", |doc| { /* edit */ }).await?;   // on disk when it returns
main.save("Mix sent to Dan").await?;                       // a checkpoint, and a fold
let mut alt = main.fork("acoustic").await?;                // open side by side
main.merge(&alt).await?;                                   // a step, "Merge acoustic"
let v = &file.versions(MAIN).await?[3];
let old = file.open_version(v).await?;                     // read-only
let back = file.restore(v, "from v3").await?;              // a new branch
```

| | How |
|---|---|
| A step | `Branch::transact`, `undo`, `redo` append the step's Loro update (its name, time and frontiers) and sync it before returning |
| Crash | the write it cut short is a torn last record; opening drops it (`DocFile::recovered`) and replays the steps after the last snapshot. `features/crdt/crdt/tests/file.rs` `kill_9_mid_edit_reopens_with_every_finished_step` kills a writer mid-edit |
| Save | `Branch::save` names a checkpoint and folds the branch into a snapshot built from the file (another handle's steps are kept); the file is rewritten beside itself and renamed over. Folded steps keep their names and places, so every one still opens |
| Branches | forked at a point (`parent`, `base` frontiers), sharing the history before it; deleting one folds its children first so they stand alone; `main` cannot be deleted |
| Kind | `meta` holds `kind`, `schema` and `id`; opening refuses another kind or a newer schema, and `open_branch` runs migrations as a step |
| Blobs | `DocFile::put_blob` keeps bytes once by SHA-256 (the name `architect::blob` uses) |
| Sync | `DocFile::persistence` is a branch as a `Persistence`, so a `CrdtDoc` (and `DocSync` on it) keeps its replica in the file |

Not yet: blobs beside the file and their reachability, an IndexedDB
`Storage`, undo that survives reopening (Loro's `UndoManager` cannot be
seeded), and one writer per file (two processes appending would
interleave).

Large media — audio a song records, images a note embeds — are blobs: by
reference, by content hash, stored beside the document (a folder next to
the file, or the store's blob table for small ones), fetched on demand
on another device, pinned to keep offline. The kind says which blobs a
document points at (`DocKind::blobs`); `architect::blob::Fetcher` reads
one from the peer the document came from or the org's server when it is
played, streaming it as it arrives; `crdt::blobs::live` is what keeps a
blob from collection ([blobs](@/architecture/blob.md)).

### Undo and transactions

Every change goes through a transaction: a person's edit, a script, an
extension, a collaborator's update arriving. A transaction is one undo
step with a name ("Split item", "Script: Arm selected"), undone as one,
and undo is local — undoing my step never undoes yours. A step can span
many transactions: a script opens one (`History::begin`), the actions it
runs make their own as always and they fold into it, and it is committed
when the script ends or aborted — taken back — when it fails
(`features/scripting/src/history.rs` `HistoryTransactions`). Scripts and
extensions get transactions ([extensions](@/architecture/extensions.md));
an engine with its own undo (Studio's) feeds its finished steps in as
transactions.

### Versions and branches

A branch is a fork of the document (Studio's alternatives: "mix B", "the
acoustic arrangement"); a checkpoint is a named point on a branch. Both
are in the file, listed, opened side by side, compared, merged back
(Loro merges them), deleted. Version history is the list of named steps
and checkpoints, any of which can be opened read-only or restored as a
new branch.

### Sync

Three ways a document reaches other devices, all `DocSync` underneath:

| | To | Over |
|---|---|---|
| **Your devices and your org** | the org's server (Task's), which keeps the canonical copy | vox over WebSocket, reconnecting; offline edits merge when back |
| **Live, peer to peer** | someone you share a link with, or a device on the same network | vox over iroh: a link (`fts-doc:<endpoint>/<doc>?key=…`, `crdt::link`), or found on the local network (`_fts._udp`, `iroh_link::LanBrowser`) |
| **Device to device** | your own devices without a server | iroh, the device registry admitting them (`crdt::p2p`) |

Presence — who is here, where their pointer is, what they have selected
— rides beside each document on every one of them. Who may read, edit,
or administer a document is the org's permission, checked on every
call: `DocPermissions` (`features/crdt/crdt/src/registry.rs`) asks the
org's permission engine about the caller the gate resolved, `read` or
`write` on `org/<org>/doc/<id>`, in `DocRegistry` and `DocsHost` alike
([collaboration](@/architecture/collaboration.md), "Who may").

### The org driver

A device keeps one driver per org that owns something on it
(`features/crdt/crdt/src/docs/org.rs`):

| Piece | What it does |
|---|---|
| `OrgDocs` | one org's `DocsDriver` (the device's index of the org's documents and their sync) and its `Placement`. `stay_connected(dial)` is the loop: dial the org's lane, run a session until a connection closes (`Caller::closed`), wait under architect's `Backoff` (full jitter, 250 ms to 10 s, reset by every connection), dial again; `Dialed::Stop` (signed out) ends it. `run_on(&lane)` is one session, for an app whose connection is already supervised (`use_app_supervised`: run it once per `Connection::generation`). `link()` is `Connecting`, `Live`, `Offline` or `Stopped` |
| `OrgSyncs` | one `OrgDocs` per org, made on first ask by the app's `open` and kept connected through its `dial`; `current(&Orgs)` is the window's org (`architect-orgs`), `follow(&Orgs)` stops the orgs the person left |
| `Placement` | each document in a document file: where the app placed it (`place(id, path)`), else `<dir>/<id>.<extension>` by its kind; synced through the file's `main` (`DocFile::persistence`), so every step that arrives is durable in the file the person opens. A new file is stamped for a document made here and left empty for one made elsewhere (`DocFile::adopt`), so it is never another lineage |

Offline, every document edits locally; the next session is a delta
both ways. Proven by `features/crdt/crdt/tests/org_sync.rs`
(`a_members_doc_syncs_through_a_dropped_connection_and_merges_offline_edits`).

Confining every service to the caller's org, with membership learnt
from the identity server, is `architect-orgs`' server side (in review,
PR #111): `DocPermissions::in_org` names a document by the same
`org/<id>/…` resource that layer keys an org's data by, so its engine
is the one to hand `DocPermissions`.

### The catalogue

Every document a person can open is in the catalogue (`Docs`): its kind,
name, parent (lineage), where it lives, when it changed — the source for
⌘O's openers ([actions everywhere](@/architecture/actions-everywhere.md)),
the library, and "recently opened".

### Extensions' data

An extension's data is a namespaced container in the document it
belongs to (the project, a track, an item), saved, undone, versioned and
synced with it.

## Files

> **Status: the core landed (2026-11-01)** in `architect-files`
> (`features/files`): the store (`features/files/src/server/mod.rs`
> `FilesServer`: every org's roots, trees, versions, labels, grants,
> uploads and quota under `OrgId::path`, bytes in the `architect::blob`
> store), the wire (`features/files/src/service.rs` `FilesService`, bytes
> over vox channels, its events per subscriber), access by the caller's
> role in the org and per-folder grants (`features/files/src/access.rs`
> `resolve`), version labels, and the client
> (`features/files/src/client.rs` `FilesClient`). Next: client sync,
> the browser and desktop integrations, adopting on-disk folders, the
> HTTP byte face; then Task and Studio move over.

Task's Files platform (`task:features/files`, about 38,600 lines) is the
file layer every app needs: roots of files and folders, versions,
chunked and resumable uploads that only send what is new, bytes over vox
channels rather than HTTP, selective sync and offline copies, search,
per-folder access and review links, devices, and a client that is clean
on wasm. It becomes **architect's**, whole — server and client — and
Task and Studio are both its clients, as any architect app is.

**Where it runs:**

- **Any architect server mounts it:** the org's server (today Task's)
  serves files for the org; a person's own machine can serve its local
  roots; a standalone files server is the same feature on its own. Its
  services are `#[architect::rpc]` traits, mounted like any others.
- **Every app is a client:** `files-client` (`ensure_root`, `put`,
  `get`, `read_range`, `resolve`, `events`), the browser and previews in
  the shell's components, selective sync and offline copies on the
  device, the same on desk, mobile and web.
- **Task** uses it for the vault's attachments and project files;
  **Studio** for songs (`session/<song>` roots), audio, stems, rig
  captures and sample packs — and every other app for whatever it keeps.

**What changes to make it architect's** (the places it leans on Task today):

| Task today | architect |
|---|---|
| a backend per org, confined to `orgs/<slug>/files/`, roles from org membership | the server's own tenancy and [permissions](@/architecture/capability.md): org, members, grants, from architect-auth and architect-permissions, which Task already uses |
| named and project versions stored as Task's Vault entities | the files feature's own version labels (its own tables), Task's vault one client of them |
| review links and shares through Task's `share` crates | review and share as part of the feature (they are file features) |
| federation and peering between Task servers | between any architect servers holding files; devices peering over iroh ([peers](@/architecture/peers.md)) |
| `files-daemon`, FUSE, File Provider, WebDAV | the same, as architect's desktop integrations (a person's files in Finder) |
| ffmpeg proxies (`files-transcode`) | a feature of the server, behind a cargo feature |

**One store with documents:** a document's blobs (above) and files are
the same store — `architect::blob`, addressed by sha256 (Task's Files
used BLAKE3; a file's `ContentId` is now the blob's sha256) — so a song's
audio is a file in its root and a blob its document points at, fetched,
pinned and kept offline the same way.

## Collaborative text

> **Status: the binding landed (2026-11-01)** in `crdt`:
> `features/crdt/crdt/src/text.rs` `TextBinding` (`on_local`, `pull`,
> `sync_into`, `caret`, `resolve`, `remote_carets`, `set_offset`,
> `set_composing`) and `EditorBinding`. Remote edits still queued when
> the person types are transformed against what they typed; the binding
> checks the editor would match the document after every hand-over and
> sends one replacement when it would not. Not yet: `editor-crdt`
> implementing `EditorBinding`, Task's notes and Studio's chart editor
> on it.

Every app has text — a note, lyrics, a chart, a comment — and every one
needs the same: a Loro text container, an editor bound to it, and the
others' carets and selections in it. Task and Studio each built it, the
same way (the shared `editor` repository, `editor-crdt`, each app's own
binding). It moves here:

- **The binding:** an `EditorBinding` an editor implements —
  `on_local(changes)` turns its edits into Loro text operations,
  `apply_remote(ops)` applies others' with an echo guard (a remote edit
  is never sent back), an offset for a prefix the editor does not show
  (a note's frontmatter). `editor-crdt` implements it; architect does
  not depend on the editor.
- **Edits are operations, not whole strings:** a keystroke is a Loro
  text op, so concurrent typing merges (Studio's chart editor sends a
  whole-string diff today, and moves to ops).
- **Carets:** each person's caret and selection, as Loro stable cursors
  (they survive others' edits), published on the document's presence
  (debounced), drawn as a coloured selection and a named caret.
- **IME and composition** cross the binding as the editor reports them.

What stays the app's: the shape of the text (a note's markdown and
properties, a chart's sections), and what the app does when typing
pauses.

## Presence and cursors

> **Status: places, rosters and the overlay landed (2026-11-01)**:
> `features/crdt/crdt/src/presence.rs` `Place` (a space and named
> coordinates), `Here` (state, pointer at 30 a second, playhead),
> `Roster` (`sync`, `here`, `pointers`, `selections_in`, `carets_in`),
> `PointerTrail`; the hooks `use_roster`, `use_here`,
> `use_presence_frame` (`features/crdt/crdt/src/hooks.rs`); the layer a
> pane draws them in, `features/workspace/src/host/presence.rs`
> `PresenceLayer` and `use_presence_overlay`. Not yet: the shared clock,
> follow, the org-wide roster, rosters keyed by an org's documents, and
> Studio's arrangement on it.

Who is here, where their pointer is, what they have selected, where
their playhead is — drawn live, in the app's own coordinates, so a
pointer lands on the same bar of the same track however differently two
people have zoomed and scrolled. Studio built it (`studio:features/session/src/sync/presence.rs`);
it moves here, and the [workspace](@/architecture/workspace.md) views'
coordinate spaces are what makes it general:

- **Keys per person, split by rate:** `state` (name, colour, what they
  have open, their selections — on change), `pointer` (up to 30 a
  second), `play` (a playhead, sent on change and extrapolated by the
  receiver).
- **A pointer is never pixels.** It is a place in a view's coordinate
  space: `{ space: "session.arrange", at: { time: 12.5, track: "Kick" } }`,
  `{ space: "chart", key: "verse-2", u, v }`. A view publishes its spaces
  (it already does, for overlays) and places a pointer in its own
  pixels; a pointer in a space this view does not show is not drawn.
- **Smooth:** each pointer is a trail of recent samples stamped with the
  sender's clock, re-anchored to the receiver's, drawn 100ms behind and
  interpolated — no jumping, no lag that grows.
- **A shared clock** (Studio's `SessionClock`, an NTP-style estimate) so
  playheads and timed pointers agree.
- **Drawn by the shell:** each pointer an arrow in the person's colour
  with their name; selections washed in it; their playhead a line — as a
  layer over the view ([overlays](@/architecture/extensions.md)).
- **Follow:** press someone's avatar to follow their view (their scroll,
  zoom and the pane they are in) until you move.
- **Who is here:** a roster (avatars, idle, what they are looking at) on
  the document, and org-wide (Task's presence strip), from one type.

What stays the app's: its coordinate spaces (time and track for a DAW),
what goes in `state` beyond name and colour, and how a selection is
drawn in a view.

## Order of work

Studio's `sync.md` order, finished here:

1. The catalogue and the document kinds (`crdt::docs`, with kind
   registration).
2. The `.session` store as architect's file format (a branch- and
   blob-aware `Persistence`), Studio's `session-format` its first user.
3. The reconcile codec into the derive; named undo and transactions.
4. Blob transfer by hash over vox and iroh, pinning, an HTTP source.
5. The org driver and the reconnect loop; caller identity in the
   registry.
6. Peer to peer: links, local discovery, the device registry.
7. Presence and cursors (Studio's, generalised to view spaces); the
   collaborative-text binding.
8. Files, whole: the store, the wire, the server feature on architect's
   tenancy and permissions, version labels of its own, the client, sync,
   the browser and the desktop integrations; Task's server and Studio
   move to it as clients.

## Open questions

1. **One file, or a package folder** (the file plus its media beside it)
   as what a person copies, mails, or puts in Dropbox?
2. **Media in the file** for small projects (one file to send), or always
   beside it?
3. **Local-network discovery:** automatic (anyone on the network sees the
   session), or only to devices you have paired?
