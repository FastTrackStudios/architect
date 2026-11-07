+++
title = "Project files: projects, assets and their sync"
description = "The spec for architect's files feature (#347): every org's projects and asset libraries held by the headless node in place, pinned per machine, browsed on demand, versioned, previewed, on what architect already has."
weight = 72
+++

This page is the spec for the files feature that replaces Task's
(`task:features/files`, 86k lines with tests, 62k without), written from the
features rather than from Task's crates
([#347](https://github.com/FastTrackStudios/architect/issues/347), and Cody's
"rewrite it, don't move it"). Task is read only to check behaviour.

**The picture.** Most storage is on starcommand. Its headless Architect node
(#400) holds every org's projects and asset libraries. A laptop or desk pins
the projects it is working on: those are full local copies that sync both
ways. Everything else stays browsable, fetched when opened and kept for
offline use. A project is a DAW session (`Right Here/Right Here.RPP` and its
`Audio Files/`), so projects are large binary trees, and a session file is
saved while the DAW holds its audio open.

## The data as it is

On starcommand, under `/mnt/starcommand/Task`:

```text
<org>/Projects/<project>/            a root: .fts-root.json {id, name}, the session, its audio
<org>/Projects/<project>/.fts-files/ Task's store: a jj repo (op_store, op_heads, index, store/chunks, store/objects), cadence.json
<org>/Assets/<kind>/                 a library: Tracks, Setlists, Signal, Music, …
<org>/Assets/Tracks/<song>/          an asset: <song>.md (type: song, artist, key, stems: [{name, path}]) and audio/, chart/, lyrics/, *.RPP
<org>/Assets/Setlists/<name>.md      an asset that is only a note (type: setlist, [[song]] links)
```

The working files are the truth. Task's own design says so (its ADR 0001:
the store is rebuildable), with one exception: a file Task dehydrated is a
pointer stub (first line `#fts-stub v1`, then `{file_id, size}`), whose
bytes are only in the store or on a peer.

## The six decisions

### 1. Storage on the headless node: in place, `.fts-files` ignored

The node **serves the tree in place**. `files_dir` in its config is the
folder above the orgs (`/mnt/starcommand/Task`). Every `<org>/Projects/<name>`
is a project root and every `<org>/Assets/<kind>` a library root. Ingesting
the tree into the blob store would need a second 11 TB on a mount that is 94%
full, so it is not an option.

- **A root's id** is the `id` in its `.fts-root.json` when there is one, so
  Task's ids carry over. Otherwise it is derived from `<org>/<section>/<name>`
  (UUID v5). The node never writes to a root it found; only a root made
  through Architect gets a marker.
- **The index** (path → size, mtime and sha-256, per root) lives in the
  node's data dir, not on the mount. Each file is hashed once. After that a
  scan is a stat walk, which is also the only change detection that works on
  NFS (inotify does not see other clients' writes).
- **Current bytes** are the working file. Past versions go in the blob store
  on the same filesystem:
  - A file up to `keep_inline` (64 MiB) is copied into the store when it is
    indexed, so every version of a session file is kept.
  - A larger file keeps the versions that sync replaced. It is renamed into
    the store, so no second copy is made.
- **`.fts-files` is ignored**: not read, not migrated, not deleted. It is
  skipped by every scan, and Cody deletes it once Architect holds the roots.
  Its history (jj commits in a custom content-addressed backend) is not
  carried over. The first index of a root is its first checkpoint,
  "Imported".
- **A `#fts-stub v1` file is never indexed as content.** It is reported as
  missing, so a stub is never synced over a real file.

### 2. Pinning on a machine

A device's **agent** is the node (`architect-desktop serve`), run on the
laptop as well. It survives the window, runs headless, and is driven over
its HTTP face and `/mcp`. The window's `FILES_*` actions call it.

- **Pins are per machine.** They live in the agent's data dir (root →
  local path) and never sync (Task's `mounts.toml`). The default path is
  `<files_dir>/<org>/<section>/<name>`, and a pin can name another.
- **A pinned root** is a full working copy that syncs both ways.
- **An unpinned root** has its manifest (every path, size, version) on the
  device, always, so it can be browsed offline. Its bytes are fetched on
  open into the agent's blob cache and kept there, so a file opened once
  opens offline. A library's notes (`.md`) are fetched eagerly.
- **The placeholder surface is WebDAV**, served by the agent on loopback
  under `/dav/<org>/<section>/<name>/…`. A `GET` of a file that is not here
  streams it from the peer while it arrives (`Fetcher::open_at`); a `PUT`
  saves a version. WebDAV works on both Linux (GNOME Files, `gio mount`,
  davfs2) and macOS (Finder's Connect to Server), with nothing installed
  in the kernel.
- **FUSE and a File Provider are later surfaces** over the same three calls:
  list a folder from the manifest, open a file through the fetcher, save a
  version.
- **Unpinning** removes the working copy only when every file in it is
  synced. Otherwise it refuses and names the files that are not.

### 3. Large binaries

- **Bytes:** sha-256, whole file, on `architect::blob`. A transfer is
  chunked and resumable: `Fetcher` reads 256 KiB frames over the
  `BlobTransfer` lane from whichever peer has the hash, runs one transfer
  per hash, and resumes from the `.part`.
- **Dedup** is by whole-file hash. A take copied into another project or
  into a library moves no bytes, and neither does a rename or a move.
- **No content-defined chunking.** A DAW writes a take once and rewrites the
  small session file. Task chunked with FastCDC but set its whole-file
  threshold to 0, so it never chunked a local file in practice.
- **Metadata:** each root's manifest is one Loro document on the existing
  peer lanes (#412: a `DocSync` bridge with each peer, the sweep as the
  fallback). It is appended to by any device and merges offline.
- **A file the DAW holds open:**
  - A file is committed only when it has **settled**: the same size and
    mtime on two scans at least 2 s apart, and the same stat before and
    after hashing (Task's "stat sandwich").
  - A file that is still moving waits for the next pass.
  - Incoming bytes go to a sibling temp file and are renamed into place, so
    the DAW sees the old file or the whole new one.
  - A file whose stat differs from the index has a local change pending and
    is never overwritten: it becomes a fork.
- **Ignored:** `.fts-files/`, `.reapeaks`, `.DS_Store`, `*.tmp`, and the
  agent's own temp files.

### 4. Assets

- **A library** is a root under `<org>/Assets/<kind>`. The kind is the folder
  name (Tracks, Setlists, Signal, Music, or anything else).
- **An asset** is one of the library's children. It is either a folder
  holding a note of its own name
  (`Praise - Elevation Worship/Praise - Elevation Worship.md`) or a note on
  its own (`Setlists/Testing.md`). The note's frontmatter is its properties
  (`type`, `artist`, `key`, `stems`), and its folder's files are its media.
- **Projects reference assets** by link, the way a setlist references
  tracks: `[[Praise - Elevation Worship]]` resolves to the asset of that name
  in any of the org's libraries. Media inside a session refer to files by
  path, as the DAW wrote them.
- **In the navigator,** a Files provider lists the org's Projects and each
  library's assets, with a pinned or online badge. An asset's page shows its
  properties, its media with a waveform each, and its history.
- **Previews:** a waveform is `Peaks`, the rendition the files feature
  already has. A WAV's peaks are read directly, and anything else goes
  through the existing `FfmpegTranscoder`. Peaks are made on the agent and
  kept by content hash.
- **Search** (`FILES_SEARCH`) covers names, paths and asset properties
  across every root the device holds.
- **Bases** come next: rows of asset properties through the `BaseRow`
  constructors.

### 5. Versions

- **Every committed change is a version** in the manifest: `{id, path,
  content, size, by, at, base}`. A delete is a version too.
- **A checkpoint** is a named set of heads. One is made automatically when a
  session file is saved (a save point: `.rpp`, `.ptx`, `.als`, `.logicx`,
  `.cpr`, `.song`, `.aup3`), at most one per scan pass. One can also be made
  on demand (`FILES_CHECKPOINT`).
- **Task's cadence** (10-minute snapshots, a checkpoint after 30 minutes
  quiet) is replaced. Versions are continuous, so an edit is never "only on
  disk", and checkpoints mark save points.
- **History in the UI:** a file's versions and a root's checkpoints.
- **Restoring** a file or a checkpoint appends versions, so it is
  non-destructive. The old bytes come from any peer that holds them; the
  node keeps them all.
- **Forks** (two devices changed one file offline) keep both:
  - the newest version wins its path;
  - the other is written beside it as `<stem> (conflict <device>).<ext>`.
  - The choice is deterministic, so every device makes the same copy, and
    heads that agree on content are settled with nothing more to do.

### 6. Orgs and permissions

- **A root belongs to the org** whose folder it is in.
- **Which devices sync an org's roots** is decided by the node's admission:
  the account's devices, and devices enrolled in the org with a role
  (#411).
- **Reading** a manifest needs any role. **Writing** needs a role whose
  capabilities (`architect_files::role_capabilities`) include writing, so
  a viewer is read-only.
- **Bytes** are served only for hashes in a manifest the device may read
  (`BlobTransferHost::allow`).
- **The org's quota** is the files feature's `Usage`: the bytes of its
  roots' heads. A version over it is not fetched by the node.
- **Per-folder `Grant`s** come with the merge below.

## Over S3

The node and a device's agent also serve the roots over the S3 API
(feature `s3`, `s3_bind`), for `rclone`, `aws s3`, `restic` and backup
tools, beside WebDAV.

- **A bucket per org.** Keys are `Projects/<project>/<path>` and
  `Assets/<kind>/<path>`: the org's tree as it is on the node's disk and
  under the WebDAV mount. A bucket per root was the other choice; it
  loses because a project's name (`Right Here`) is not a valid bucket
  name, because a key and a role belong to an org and not to a root, and
  because a copy from a project into a library would cross buckets
  rather than move no bytes.
- **Versions are the manifest's.** `PutObject` and a completed multipart
  upload are `Engine::save_reader`, as a WebDAV `PUT`; the version's id
  is the S3 version id, a delete is a delete marker, and
  `ListObjectVersions` is a path's history. History is append-only, so
  deleting one version is refused, and versioning cannot be suspended.
- **Keys and permissions.** An access key belongs to a person or a
  device, in one org, with a role, and is issued on the node. Each
  request lowers that role to what the holder holds now (a device's
  grant in `Admission`), then runs the files feature's own check
  (`role_capabilities`, `access::resolve`). `SigV4` is `s3s`'s.
- **Bytes** come from the device's store or working copy, else are
  streamed from a peer as they arrive, as a WebDAV `GET` does.

## Each feature: what it reuses, what is new

| Feature | Reuses in architect | New |
|---|---|---|
| Roots, ids, paths | `features/files` `RootId`, `VersionId`, `ContentId`, `RootPath` (no `..`), `Version` | the layout scan (`<org>/Projects`, `<org>/Assets/<kind>`, `.fts-root.json`) |
| Bytes, dedup, resume | `architect::blob` `LocalFs`, `Fetcher`, `BlobTransferHost`, `VoxRemote` | a `BlobStore` that also answers from indexed working files |
| Metadata sync | `crdt` `DocRegistry`, `CrdtDoc`, the peer catalogue and `DocSync` bridges (#412), the node's sweep | the manifest document (versions, heads, forks, checkpoints) |
| Two devices and a node, offline | the node (#400), its iroh lanes, `Admission` (#411), Loro merge | none |
| The sync agent | `architect-desktop serve`, its config, `#[architect::service(actions)]` for the HTTP face and `/mcp` | the engine: scan → versions, manifest → working copy |
| Pinning, per-machine paths | the node's data dir | pins file, unpin guard |
| Placeholders, fetch-on-open | `Fetcher::open_at` (streams while it arrives), `EngineHost::dav` | a small WebDAV face over the manifest |
| Renditions, waveforms | `features/files` `Peaks`, `PeakReducer`, `Transcoder`, `FfmpegTranscoder` | a WAV reader for peaks without ffmpeg |
| Assets | vault frontmatter parsing, `[[links]]` | asset discovery in library roots |
| Search | navigator search over provider items | `FILES_SEARCH` over manifests and asset properties |
| Navigator, previews, history UI | `NavigatorProvider`, `linked::Builder::navigator`, the extension views (`extensions/projects` is the model) | a Files provider, a root/asset page with waveforms and versions |
| Orgs, roles, quota | `role_capabilities`, `Usage`, the org lane | none |

What does not come over from Task, and why:

- `files-store` and `files-storage`: `architect::blob` replaces them.
- `files-sync`: the crdt lanes replace it.
- `files-daemon` and `files-daemon-proto`: the node and the service macro
  replace them.
- `files-client`: the agent's HTTP face replaces it.
- The jj repo, its op log and its compaction: the manifest document replaces
  them.

## The design, in the smallest pieces

`architect-files` gets a feature `sync` (native), next to `server`:

- **the manifest:** one Loro document per root. It holds a `meta` map (id,
  org, section, name), an append-only `versions` list and a `checkpoints`
  list. Every query is pure over the list: a path's heads, its history, a
  checkpoint's paths, forks. Appends never conflict, so two devices' offline
  edits both survive the merge.
- **the index and scanner:** walk, stat, settle, hash with the stat
  sandwich, and emit versions.
- **the store:** `LocalFs`, plus a map from hash to the working file that
  holds it.
- **the engine:** one pass per root, scan first (Task's trap: capture
  before pulling), then apply the heads to the working copy, then make the
  conflict copies.
- **the agent service:** `FilesAgent`, with `roots`, `browse`, `pin`,
  `unpin`, `history`, `restore`, `checkpoint`, `checkpoints`, `peaks`,
  `search` and `status`. It runs as actions on the node's Action List and as
  its HTTP face for the window.
- **WebDAV:** `OPTIONS`, `PROPFIND` (depth 0 and 1), `GET` with `Range`,
  `PUT`, `MKCOL`, `DELETE` and `MOVE`, over the manifest and the fetcher.
- **assets:** library children, their notes' frontmatter, link resolution.

The node (`apps/desktop/src/node`) gains:

- a `files_dir`;
- the manifests in its catalogue;
- `BlobTransfer` on its peer lane;
- the engine loop;
- the agent service.

The window gains a Files navigator provider and its pages.

**Merging with `FilesServer`:** `FilesServer`'s `State` stays the
server-held root, used by attachments and browser uploads. A synced root is
the same `Root` and `Version` held in a document instead, so any peer writes
it. The browser's byte face (`WebFiles`, tickets, renditions) reaches synced
roots once `FilesServer` reads them through `Acting`; that is the follow-up
that leaves one set of roots.

## Line budget

| Piece | Lines |
|---|---|
| Manifest document and queries | 400 |
| Index, scanner, settle | 300 |
| Store over working files | 150 |
| Engine (apply, forks, checkpoints) | 450 |
| Layout, ignore rules, root ids | 150 |
| Agent service | 350 |
| WebDAV | 400 |
| Assets, search, peaks | 300 |
| Node wiring | 300 |
| UI: navigator provider, pages | 600 |
| Tests | 700 |
| **Total** | **≈ 4,100** (budget 5,000), against Task's 86k |

## What the stack built

| Layer | Code | Tests |
|---|---|---|
| Store and sync engine | `features/files/src/sync/manifest.rs`, `features/files/src/sync/scan.rs`, `features/files/src/sync/store.rs`, `features/files/src/sync/engine.rs`, `features/files/src/sync/lane.rs`; `apps/desktop/src/node/files.rs`; `VaultCollab::with_plain_docs`; `crdt::p2p::node_router_with` | `features/files/tests/sync.rs`, `apps/desktop/tests/files.rs` |
| Pinning and mounts | `features/files/src/agent.rs` `Files`, `features/files/src/sync/agent.rs`, `features/files/src/sync/dav.rs` `Dav` | `apps/desktop/tests/files.rs` `an_unpinned_project_browses_over_webdav_and_a_file_opens_on_demand` |
| Assets | `features/files/src/sync/assets.rs` (`assets_of`, `frontmatter`, `wav_peaks`, `Engine::search`, `Engine::peaks`) | `features/files/tests/sync.rs` `a_library_s_assets_browse_search_and_draw_from_a_laptop_that_holds_none_of_it` |
| S3 face | `features/files/src/sync/s3/mod.rs` `S3Face`, `features/files/src/sync/s3/keys.rs` `S3Keys`, `apps/desktop/src/node/s3.rs` | `features/files/tests/s3.rs` |
| UI | `features/files/extension/src/lib.rs`, `features/files/extension/src/nav.rs`; `apps/ui/src/files.rs` (`FILES_*` over `ARCHITECT_FILES_AGENT`) | the extension's unit tests; the live check |

The live check ran on a scratch copy of one project (Right Here, 1.6 GB)
and one asset folder (a Tracks song, 2.1 GB): a node holding the tree in
place, a laptop node pinning the project (1.6 GB in about 3 s on loopback),
a session save there reaching the node in 4 s and a new file on the node
reaching the laptop in 6 s, unpin, the project mounted with `gio` over
WebDAV and a take opened on demand (86 MB in 161 ms, kept for offline), and
the window's Files pages with a waveform per stem.

## What is left after the stack

- FUSE on Linux and the macOS File Provider, over the engine's three calls.
- `FilesServer` reading synced roots, so browsers reach projects by
  ticket, and per-folder grants.
- Bases rows for assets.
- Pinning a project pins the assets it links.
- Background hashing order and progress for the first index of 11 TB (a
  root is indexed whole in one pass today).
- Empty folders are not synced (the manifest holds files).
- Two devices close a fork each, so a fork leaves two heads with the same
  bytes; they settle, but each pass reads both.
- Installing the agent with launchd or systemd.
- Federation (#194's second item).
