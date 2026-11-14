+++
title = "Deploy: Architect, headless"
description = "architect-desktop serve: the Architect app run with no window as an always-on node, a sync peer for the org's vaults that serves Sites, share links, MCP and the HTTP face, released as a pure-Nix OCI image."
weight = 75
+++

There is no separate Architect server. A deployed "server" is the Architect
app run headless: the same app and the same node, with no window. The desktop
and mobile apps are peers. The account server only introduces them (accounts,
orgs, the device address book and pairing, [devices](@/architecture/devices.md)).
A headless Architect is one more device among them, the one that is always
online. Every other device syncs with it the way it syncs with any peer, and
they all keep working while it is stopped.

## Run it

```sh
cargo run -p architect-desktop -- serve --config architect.toml
# without the window's crates at all, as the image builds it:
cargo run -p architect-desktop --no-default-features --features vox,node-wasi -- serve
```

`serve` is a run mode of the app's own binary (`apps/desktop/src/main.rs`),
beside `actions`, `mcp` and `--headless`, so it is one product with one
binary and one version. Built with `--no-default-features --features vox`
(no `gui` feature), the binary is the node alone: no Blitz, wgpu or wayland,
and no GPU libraries in the image. `node-wasi` adds Wasmtime, so installed
WASI components draw a share's blocks ([below](#blocks-extensions-draw)). The desktop build is unchanged.

## What it does

`apps/desktop/src/node/mod.rs` `start`:

| | |
| --- | --- |
| **On the account** | With an issuer and a session it registers itself in the account's address book as a `server` device at its iroh endpoint, offers its org (`ARCHITECT_ORG`) if the account shares none yet, and lets the account's other devices in (`architect_devices::account::pass`). It runs again every five minutes, and on every sweep until another device is on the account. |
| **In an issuer org** | With `ARCHITECT_HOST_ORG` it hosts the org's enrolments and its vaults for signed-in web clients (below); with `ARCHITECT_ENROL_AT` it enrols itself in an org another node hosts. Its vaults are that org's. |
| **Its lanes over iroh** | The node's documents (`crdt::p2p::node_router`: `DocSync` and `DocPeer`), served to the devices its `Admission` lets in. A revoked device's connections close (`architect_devices::serve`). |
| **A sync peer for the vaults** | Each markdown file of each vault is a document (`vault_collab::VaultCollab`, written behind to the file; an edit on disk is merged in). `apps/desktop/src/node/catalog.rs` `VaultCatalog` names them on the peer lane. With each peer it dials, the node keeps a live `DocSync` bridge per note (below), so an edit reaches the others within a second. Every `ARCHITECT_SWEEP_SECS` it also sweeps each peer (`crdt::peer::pull_with`), the fallback for what a bridge missed. |
| **Sites** | Each vault in `ARCHITECT_SITES` is built as a Site ([sites](@/features/sites.md)) and rebuilt when the vault changes. |
| **Share links** | `/s/<code>/`: a share's scope of a vault, opened in the web app as a guest of the link, to the audience it names, until it expires or is revoked ([below](#share-links)). |
| **The HTTP face and vox** | `apps/desktop/src/node/service.rs` `Node` (`status`) over same-origin vox at `/vox`, its HTTP face (`POST /node/status`) and the Action List (`NODE_STATUS`). |
| **MCP** | `/mcp`, only when `ARCHITECT_MCP_TOKEN_FILE` holds a bearer: `NODE_STATUS` and the `NOTES_LIST`, `NOTES_READ`, `NOTES_WRITE` and `NOTES_CREATE` actions (`apps/desktop/src/node/service.rs` `Notes`). An agent writes a note on the always-on node, and every device that syncs with it gets the note. |
| **Health** | `GET /health` answers `ok`; `GET /version.json` gives the build. |

## Configure it

A TOML file (`--config`, or `$ARCHITECT_CONFIG`), then the environment over
it (`apps/desktop/src/node/config.rs` `Config`). Secrets are files mounted
where the node reads them, never values in the environment.

| Variable | Default | What |
| --- | --- | --- |
| `ARCHITECT_NAME` | `architect` | the device's name on the account |
| `ARCHITECT_BIND` | `0.0.0.0:8080` | HTTP: `/health`, Sites, `/mcp`, `/node/status`, `/vox` |
| `ARCHITECT_DATA_DIR` | `architect-data` (`/data` in the image) | its iroh key, device id, documents and built Sites |
| `ARCHITECT_VAULTS` | none | `id=path,…`: the vaults it holds (a relative path is under the data dir) |
| `ARCHITECT_SITES` | none | `/mount=vault,…`: vaults published as Sites |
| `ARCHITECT_EXTENSIONS` | `<data dir>/extensions` | installed extensions, a package folder each (`<id>/extension.toml`): the fences they claim are drawn on share links and Sites ([below](#blocks-extensions-draw)) |
| `ARCHITECT_ISSUER` | none | the account server, `https://auth.fasttrackstudio.app` |
| `ARCHITECT_SESSION_FILE` | none | a file holding the account's session token |
| `ARCHITECT_ORG` | none | the org it offers the account while the account shares none |
| `ARCHITECT_HOST_ORG` | none | an issuer org (slug) the session is in: the node serves its enrolment lane at `/org/{slug}/vox` |
| `ARCHITECT_ENROL_AT` | none | another node's HTTP origin: this node enrols itself there, once |
| `ARCHITECT_ENROL_ORG` | none | the issuer org (slug) it enrols in at `ARCHITECT_ENROL_AT` |
| `ARCHITECT_ENROL_ROLE` | `member` | the role it asks for, at most the session's own in the org |
| `ARCHITECT_PEERS` | none | `endpoint@host:port,…`: nodes it dials directly |
| `ARCHITECT_APPLE_APP_IDS` | none | `<team id>.<bundle id>,…`: the iOS apps its share links open in as universal links ([below](#a-link-opens-architect)); none: no `apple-app-site-association` |
| `ARCHITECT_WEB_APP` | none (`beside` with the chart's web front end) | whether the web app is served beside the node, on its origin: `beside` (a proxy serves the same release's web bundle at `/app/`), or that bundle's folder (checked at start). A share link's page is the web app's page, which loads it from there ([below](#routing)); none: a link's page says it opens in the web app, which this server does not serve |
| `ARCHITECT_IROH` | `n0` | `n0` (relays and lookup) or `local` (loopback only, for tests) |
| `ARCHITECT_IROH_BIND` | any port (`0.0.0.0:7777` in the image) | the iroh UDP socket |
| `ARCHITECT_MCP_TOKEN_FILE` | none | `/mcp` is served only with this bearer |
| `ARCHITECT_SWEEP_SECS` | `30` | how often it sweeps its peers, the fallback for what a live bridge missed |
| `ARCHITECT_LIVE_MAX` | `256` | how many notes it keeps live with each peer; `0` sweeps only |
| `ARCHITECT_FILES_STORE_MAX` | a quarter of the filesystem under the data dir, at most 100 GiB (75% of the data volume with the chart) | the hard limit on the files' blob store, `<data dir>/files/blobs`: bytes, `16Gi`/`500M`, or `40%` of that filesystem ([below](#the-blob-store-s-budget)) |
| `ARCHITECT_FILES_KEEP_FREE` | 15% of that filesystem, at most 64 GiB | free space the store (and a holding node's tree-side areas) never writes into, whoever else filled the disk; `100%` is refused |
| `ARCHITECT_FILES_HISTORY_MAX` | a tenth of the tree's filesystem, at most 500 GiB | a holding node's kept versions, beside its tree: at the limit a replacement waits ([below](#the-blob-store-s-budget)) |

The other `ARCHITECT_FILES_*` and `ARCHITECT_BLOB_*` settings (the files
engine, the drive, a bucket) are in `apps/desktop/src/node/config.rs`'s
module docs.

### The blob store's budget

Never losing a byte comes first; a bounded store comes second. Where the two
conflict, the node defers or refuses and says so. It never deletes to make
room.

- **The store** is `<data dir>/files/blobs` on the data volume. Every write
  into it is checked first against `ARCHITECT_FILES_STORE_MAX` and against
  `ARCHITECT_FILES_KEEP_FREE` on the filesystem under it (`statvfs`). A write
  that would cross either is refused. A file that has to be fetched waits and
  is tried again on every pass. A file larger than the whole limit is
  reported as *too large*: it arrives only once the limit is raised. If the
  free space cannot be read, at start or later, writes that need the floor
  are refused (closed, not open).
- **A holding node** (`ARCHITECT_FILES_HOLD`) keeps almost nothing in that
  store.
  - Its fetches land beside its tree, in `<files_dir>/.architect/incoming`,
    and are renamed into place, so a 30 GiB file needs no room on the data
    volume.
  - Every version it replaces or deletes is kept in
    `<files_dir>/.architect/versions`, on the tree's own disk, unless that
    area or the store already holds those bytes. Another working file with
    the same bytes does not count: a working file can change. Deletes are
    the moves' old names, a peer's delete, a WebDAV or S3 DELETE, and a
    checkpoint restore that removes files. They are applied after every
    write in the same pass, so a move writes its new name before its old
    one goes.
  - That area is bounded by `ARCHITECT_FILES_HISTORY_MAX` and by the tree
    filesystem's floor (`ARCHITECT_FILES_KEEP_FREE` again). The default is a
    tenth of the tree's filesystem, at most 500 GiB, worked out at start from
    the share's `statvfs`: on a pooled export whose reported size changes,
    it can change between restarts.
  - At either bound, replacements and deletes wait. They are reported as
    `history_full` (in the root and in `files_store`), with one
    `files: kept versions are at their bound` warning that carries the
    history area's numbers. Nothing frees history by itself: raise the bound
    or make room. Nothing is deleted.
  - A version is kept by a hard link taken right before its file is swapped
    out, so it costs no copy. Once the swap has happened, the link is the
    file's only name, and nothing can write to it in place. A file that
    already has another name (a user's hard link) is never linked: it is
    cloned where the filesystem can (`FICLONE`: btrfs, XFS), else copied and
    verified, so a write through the other name never reaches the kept
    version.
  - A run that dies between the link and the swap leaves a shared file. At
    start, before any write, every store file with another name is
    re-hashed: changed bytes go to `<data dir>/files/corrupt/` (or the area's
    `.corrupt/`) and the claim is withdrawn; unchanged ones are given an
    inode of their own, and the working file that shared it is known again
    by its new ctime.
- **`.architect` on the share.** It is visible to anyone who mounts the
  share's root. It holds `incoming/` (bytes about to be renamed into the
  tree), `versions/` (previous versions by sha-256: for some old versions the
  only copy) and a `README.txt` saying so, which is also the node's mark.
  - The mark is written to a temp name and renamed in. An empty folder (a
    claim a crash cut short) is claimed again.
  - If `.architect` exists and is not the node's own (a file, a link, or a
    folder with other things in it and no mark), a holding node refuses to
    start and names what to move.
  - **A share the node cannot write** (read-only, root-squashed): the node
    starts and serves reads, keeps no history, and every replacement or
    delete waits. It reports `history_error` ("cannot keep history: … cannot
    be written") and logs one warning at start. Nothing is written to the
    tree, so nothing can be lost; making the share writable resumes it.
- **Eviction.** Over 90% of its limit, the store drops only blobs that a
  working file on the same device holds, re-hashed just before the delete,
  and only blobs that are no old version anywhere. It never relies on
  another holder's copy (#530).
  - Eviction holds the same lock as a pass, so a write that decided "the
    store keeps these bytes" never races it.
  - It re-reads at most 2 GiB of working files per pass and logs its
    progress, so a node that starts over budget (one that ran an older build
    that copied the tree) catches up over several passes without stalling
    sync.
  - **The store copies it drops this way are what protected small files
    against direct edits.** Once one is dropped, a later direct edit of that
    file loses its previous version.
- **Writes are staged.** Every file the node writes (a fetched version, a
  conflict copy, a WebDAV or S3 write applied to the tree) is staged beside
  its target as `<name>.architect-part`, recorded first in the node's
  journal (`<data dir>/files/pending-writes.json`).
  - A copy is synced, read back and checked against its sha-256; on a
    holding node it counts against the tree filesystem's floor while it
    exists.
  - The target is checked again just before the swap: a file the user
    changed meanwhile is never overwritten, and the next scan makes the two
    a fork with a conflict copy.
  - At start, only temp files the journal names are removed. Any other file
    ending in `.architect-part` is a user's: left alone, never synced, and
    listed in the root's `strays`.

**Which edits keep their previous version.**
- **Kept:** the edits the node makes itself (a newer version from a peer, a
  restore, a WebDAV or S3 write), and every delete it applies (a move's old
  name, a peer's delete, a WebDAV or S3 DELETE, a checkpoint restore).
- **Not kept:** edits made directly on the tree behind the node's back (a
  DAW on the NFS share, `rsync` into it). The node sees such an edit only at
  its next scan, when the old bytes are already overwritten on disk. The
  exception is a file whose bytes the node still has a store copy of; an
  older build copied every small file, until space pressure drops that copy
  (above).
- Such a version stays in the list, and `FILES_HELD` (`Files::held`) says
  whether anyone holds its bytes.
- A device that pins roots keeps every file up to 64 MiB in its store as it
  commits it, as before.

`POST /node/status` (`NODE_STATUS`) answers `files_store`:
- bytes used and in flight, the limit and the floor, the filesystem's free
  bytes and size;
- files deferred now, files too large, replacements and deletes blocked on
  history (`no_history`, `history_error`, `history_full`), and writes
  refused since start;
- on a holding node, `history_used`, `history_max`, `tree_fs_free` and
  `incoming_used`.

When a root starts waiting for room, the node logs one
`files: fetches deferred for space` event with the same numbers. A fetch
refused for space ends its `blob.fetch` span with `store.need`, `store.used`,
`store.max`, `fs.free` and `fs.keep_free`. At start, the node logs each area's
budget and where each number came from (`store.max_from`,
`fs.keep_free_from`). A limit derived from a volume that reports the host's
disk is visible there.

```toml
name = "fasttrackstudio"
issuer = "https://auth.fasttrackstudio.app"
session_file = "/run/secrets/architect/session"
org = "fasttrackstudio"

[vaults]
docs = "vaults/docs"

[sites]
"/" = "docs"
```

## Live sync

`apps/desktop/src/node/live.rs`: for each peer it dials, the node holds one
iroh connection and on it one `crdt::peer::bridge` per note either side
holds, up to `ARCHITECT_LIVE_MAX`. A bridge is two ordinary `DocSync`
sessions, one on this node's registry and one on the peer's, through an
in-memory pipe: what either side publishes goes to the other as soon as it
arrives, and every editor attached to either registry sees it. The node
re-lists the peer's notes every few seconds, so a new note gets a bridge; a
note the peer holds and this node does not is made here.

It bridges the peers it is configured with (`ARCHITECT_PEERS`) and, of those
the lookup finds, the ones whose endpoint id is higher than its own, so two
nodes that each know the other open one set of bridges. The sweep reaches
the rest, and catches up whatever a dropped connection missed.

A file edited on disk reaches the note's document at once (`vault_collab`
merges it in), and the peer writes its file after the write-behind (about a
second). `apps/desktop/tests/node.rs`
`an_edit_reaches_the_other_node_live_within_a_second` measures the document,
with sweeps an hour apart.

**A vault seeded on two nodes.** Two nodes that each made a note from their
own copy of the file hold two documents of one id
(`crdt::peer::FOREIGN_LINEAGE`), and syncing them as they are would put every
line in twice. The bridge or sweep that meets them settles it
(`crdt::peer::adopt_lineage`): the second seed adopts the first's lineage.
Both histories are merged and reverted to the first seed's text in one
change, so the note is one document again and reads as the first seed did.
"First" is the history with the oldest first change, then the lower peer id,
so every node picks the same one, and two nodes settling at once write the
same change. If the two copies said the same thing (one vault copied twice),
nothing is lost. If they differed, the second copy's text is kept only in the
history.

## An org's devices

The account's own devices find each other through the issuer's address book.
An org's other members' devices are enrolled at the node that hosts the org
(`apps/desktop/src/node/org.rs`):

- **Hosting.** With `ARCHITECT_HOST_ORG = "<slug>"` the node asks the issuer
  for that org's id (and keeps it in its data dir, so it starts with the issuer
  down) and serves `architect_devices::server::EnrolmentHost` at
  `/org/<slug>/vox`. A caller presents an issuer session as its bearer, or (the
  web app) the issuer's access token of a first-party client: the node resolves
  it with the issuer's `whoami` (`apps/desktop/src/node/org.rs`
  `IssuerSessions`), which takes either, and verifies no JWT itself, and learns the person's orgs and role from the issuer
  (`architect_orgs::server::MembersResolver`). A member enrols a device with a
  role no higher than their own, an owner or admin lists them all and anyone
  lists their own, and the one who enrolled it or an owner or admin revokes it.
- **What an enrolled device may do.** An enrolment is a `Grant::Org` in the
  node's `Admission`, kept in `devices.json` in the data dir. Its iroh calls
  are let in, and it opens and sweeps the vaults' notes when its role may
  write them (`crdt::p2p::NodeAccess::org_docs` over the org's
  `ConfinedEngine`, `apps/desktop/src/node/org.rs` `engine`): a viewer
  connects but opens nothing, a member or above syncs. A revoke refuses the
  device's next call and closes its connections.
- **Enrolling a node.** With `ARCHITECT_ENROL_AT = "https://<host node>"` and
  `ARCHITECT_ENROL_ORG = "<slug>"`, a node signed in as a member enrols its own
  endpoint there as a `server` device with `ARCHITECT_ENROL_ROLE`. It learns the
  host's endpoint from its `/node/status`, lets the host in as the org's, and
  syncs with it like any peer. It enrols once (it leaves an `enrolled` marker
  in its data dir), so a revoke holds.
- **From the app.** `ORG_DEVICES`, `ORG_ENROL` and `ORG_REVOKE`
  (`features/accounts/src/actions.rs` `register`) do the same as the signed-in
  account, in its current org, at the host given as `host` (kept per org in
  `accounts.json`). The Account menu's Organization section opens the same
  page ("Devices in <org>…").

- **Signed-in web clients.** The same lane carries the vaults: `Notes` (list
  and read for any member, write for a member or above) and `DocSync` on the
  vaults' registry. A web app signed in at the issuer opens
  `wss://<node>/org/<slug>/vox` with its session as the bearer, lists the notes
  and edits one live (`crdt::sync::SyncedDoc` on `collab_doc_id(vault, path)`),
  the way a device does over iroh. A member or above opens a note live; a viewer
  reads it through `Notes` (`apps/desktop/src/node/org.rs` `OrgLane::opens`).
  No session, or a session not in the org, is refused at the gate.

```toml
# the org's always-on node
host_org = "fasttrackstudio"

# a member's node elsewhere
enrol_at = "https://architect.fasttrackstudio.app"
enrol_org = "fasttrackstudio"
```

## Share links

A share link puts part of a vault on `https://<node>/s/<code>/`, the way you
share a topic with someone (#374). Every link is under `/s/`
(`features/spaces/src/share.rs` `LINKS`), so a reverse proxy sends one
prefix to the node and a code never clashes with the web front end's paths
(#431). The model is #197's share links
(`features/spaces/src/share.rs` `Share`):

| | |
| --- | --- |
| **Scope** | the whole vault, a folder (the link is rooted there), a tag, a list of notes, or the notes a `.base` file selects (`ShareScope`) |
| **Code** | 16 random characters of base32, 80 bits (`code_from`): the segment after `/s/` |
| **Audience** | `public` (anyone, may be listed), `link` (anyone with the link, `noindex`), `org` (the org's members, signed in at the issuer), `invited:<emails>` (those people, signed in) (`Audience`) |
| **Access** | read-only, or allowed to comment (`ShareAccess`) |
| **Expiry** | an optional time, after which the link is `404` |
| **Revoke** | final; the link is `404` from the next request on, and its pages are deleted |
| **Domain** | optional: the same share at the root of a host of its own |
| **Audiences** | optional: named slices of the scope, each a filter on a link of its own ([below](#one-share-many-audiences)) (`ShareAudience`) |

### A link opens Architect

A share link is Architect itself (#629, #646): a person with the app
opens it there, as a guest of that link, and a browser opens the web app
as a guest of it. The node renders no note of a link to HTML for anyone
to read.

- **In a browser.** `GET /s/<code>/` and every page path below it
  (anything ending in `/`, a page's folder, and `_files/`) answer the web
  app's own page (`apps/web/app/index.html`, compiled into the node:
  `apps/desktop/src/node/share_shell.rs`), with a `<head>` for link
  previews (`<title>`, `description` and `og:*` from the link's title,
  only once the link is open to the caller; `robots` `noindex` unless the
  link is public), one `<noscript>` line, and the app's own
  Content-Security-Policy as a header (`share_shell::policy`, the one
  `scripts/web-routes.md` gives, with the node's issuer). The page loads
  the app from `/app/start.js` on the same origin; the app reads the code
  and the path from its location (`apps/web/src/lib.rs` `share_route`)
  and draws the link from its guest service (`apps/web/src/app/guest.rs`
  `GuestPage`): the link's notes in their sets (headings over lists in
  the link's root note, `apps/web/src/guest_view.rs` `sections`), each
  with its art and a play button for its recording (a root setlist an
  island claims: its view first, the notes after it under "Notes on this
  link"); a note as the view its type's island claims, handed what the node gives a guest, then its
  page's words and images; the link's recordings on the Now Playing bar
  (streamed from the link's own file URLs); its files and "Download all";
  a rebuild or a revoke read again through `_live`. The page path and
  the link are checked as before (`Shares::open`, `admit`): expired,
  revoked, unknown, a page another audience holds or one that is nowhere
  is the same plain `404`; a link for signed-in people answers the app's
  page with `401` or `403` and names nothing of the link, and the app
  offers the node's sign-in (`/s/_signin`). The guest service takes that
  sign-in's cookie from a page on the node's origin when a call carries
  no token, and the app sends this browser's own session when it is
  signed in (when the node says to sign in to that session, one it no
  longer knows, the app asks again without it, so the cookie counts).
  Signed in, as anyone, the app still opens a link as its
  guest; "Sign in" in its account menu goes to your own vaults (`/app/`).
- **Who serves the page.** The node, for `/s/<code>/…`; the bundle's
  `/app/start.js`, `/app/style.css` and `assets/app-<hash>/` come from
  whatever serves the web bundle on the same origin. In production that
  is the reverse proxy in front of both (the chart's Ingress, Caddy):
  `/s` to the node, the rest to the bundle, as before. With `just dev`
  the web app's dev server proxies `/s` to the node and writes its own
  `/app/start.js`, which starts its build (`xtask/src/dev/mod.rs`
  `DEV_START`). A node with no web bundle beside it (no
  `ARCHITECT_WEB_APP`) answers a plain `503` saying the link opens in the
  web app, which this server does not serve ([Routing](#routing)).
- **A share with a domain of its own** (`ShareChange::Domain`) stays a
  Site at that host's root: its built pages, as `ARCHITECT_SITES` are
  served. No web bundle and no `/s/<code>/_guest` are on that host, and a
  domain is a publication. Its `/s/<code>/` link opens the web app like
  any other.

- **The guest service.** The app reads a link through `ShareGuest`
  (`features/spaces/src/guest.rs`), a read-only host service at
  `POST /s/<code>/_guest/<method>`: `link` (its title, layer, audience
  and notes in list order), `read` (one note as plain blocks of what its
  page says: `GuestNote`, `PlainBlock`), `files` (its files at the
  link's layer). Each call is checked by the same two functions as
  every request for the link's pages and files
  (`apps/desktop/src/node/shares.rs` `Shares::open`, `admit`): live,
  the audience, the audience's filter now, the layer. A note always
  comes as what its page says, built from the page's own sanitised
  prose (`features/sites/src/plain.rs` `blocks`, written as
  `<page>/_note.json` in the build's data folder beside the served one,
  `architect_sites::data_dir`, so no URL of the link, however a
  filesystem folds its spelling, reaches it: it is read only through
  the service).
- **What a guest's view reads (#648, #669).** A guest gets what the
  sharer shared. `read` hands a note's text (`GuestNote::source`) only
  when the island this node draws its type with reads the note (its
  claim names `documents` and its extension declares it,
  `features/sites/src/island.rs` `note_reads`), and the link's images
  and lyrics (`GuestNote::files`: the note's own first, then the rest of
  the link's, within `architect_spaces::view::within_caps`) only when
  that extension declares `files` too. The node decides it, never the
  caller. The text is the one the share's build wrote for that link
  (`features/sites/src/guest_text.rs`), from the same link resolution as
  the page: `%%comments%%` and raw HTML gone, a link to anything outside
  the link reduced to its words, an embed of it or of a held-back file
  gone, a reference definition that points outside gone, frontmatter
  never. A share's build follows no symlink (#674), so no symlink's
  target is ever shared. Its answers are `private, no-store`, `no-referrer`, and
  any origin may call it without credentials (`Access-Control-Allow-Origin: *`):
  the code is the credential, and a session travels in the body, never a
  cookie (the node's own sign-in cookie stands in for an empty token on
  its own origin). It is the one way an app, the web app or an
  extension's view reads a link.
- **Universal links.** With `ARCHITECT_APPLE_APP_IDS` set, the node
  serves `/.well-known/apple-app-site-association` naming those apps for
  a link's pages alone (`/s/*/`): never `/s/_*` (its sign-in), a name
  the node reserves (`/s/*/_*`: the guest service, `_files/`,
  `_archive.zip`, `_live`), or a file (no trailing `/`: media, a PDF, an
  image), so "Download all" downloads
  (`apps/desktop/src/node/shares.rs` `apple_app_site_association`). The
  App Store build claims FasTrack's own domain; any other node's links
  open in the app through `architect://open?url=<the link>`
  (`features/spaces/src/guest.rs` `app_link`).
- **In the app.** A link opens in the **Shared** tab
  (`apps/ui/src/guest.rs` `GuestPage`): the sharer's node by name, who
  it is shared with, "Read only"; never one of your vaults. It comes from
  `SHARE_OPEN {url}` (⌘K "Open a share link…", a pasted link, an
  agent) or from the OS (`incoming`: a universal link, the
  `architect://` scheme, a second launch's hand-off, the launch's own
  arguments). A note is drawn by the extension that claims its type
  (`Embeds::resolve`) once a note's text is given to guests; until then
  as its page's words, with "Open in browser". The extension draws a
  guest's content only through its page, on a host that runs no action,
  makes no host-service call and keeps its settings and shared state
  apart (`features/pages/src/host.rs` `AppHost::guest`): it reads and
  writes nothing of yours. A linked or process view, which runs with
  your own session, never draws it. On a phone the tab takes the bar's
  fourth place while a link is open (`Tab::kept`). The account's session is
  sent only to a node the account already uses for an org of its own
  (`trusts`). A revoke or an expiry empties the tab on the link's next
  news (`_live`) or within five minutes.
- **How a link reaches the app.** iOS: the scene delegate
  (`apps/mobile/src/scene.rs`) hands over a universal link's activity
  and the scheme's URL contexts, at a cold start and later; the
  scheme is in `apps/mobile/ios/Info.plist`, the domains in
  `apps/mobile/ios/Architect.entitlements` (signed in by
  `apps/mobile/ios/deploy-testflight.sh` only when the profile allows
  Associated Domains). macOS: `CFBundleURLTypes` in the bundle
  (`scripts/desktop/macos.sh`), the open URLs reaching the presence's
  hand-off. Linux: the AppImage's desktop entry handles
  `x-scheme-handler/architect` (`scripts/desktop/linux.sh`); a build run
  from a folder registers itself with `architect-desktop links register`
  (`apps/desktop/src/links.rs` `register`). A second launch hands its
  link to the running app (`architect_presence::single_instance`).
- **What the app opens.** `ShareUrl::parse`: `https:` only, or `http:`
  to this machine; no credentials; a share link's path. A link the OS
  hands the app, or an agent's `SHARE_OPEN`, asks the person first
  (`asks_first`) when it points at this machine or a private network
  (`ShareUrl::is_local`, or a name that resolves there), or at a node
  that is neither `architect.fasttrackstudio.app` nor one of the
  account's ("Open a shared link from <host>?"); nothing is fetched
  before the yes. A link the person types or pastes opens directly. A
  name checked as public is read from the addresses it was checked at,
  so the DNS can't move it afterwards. An image is fetched only when its
  URL, parsed and normalised, is on the link's node below the link.

### Blocks extensions draw

A Site's pages draw the fences extensions claim, as the app draws them
in a note (#488): a ```` ```kf ```` fence is the chart, not its source. A
share's build draws them too, and a link's guest reads a drawn block as
its words and its claim (`architect_spaces::BlockKind::Drawn`); in the
web app the extension that claims the note's type draws it (#646). The
node asks, in this order:

1. **The extensions installed on it**, a package folder each under
   `ARCHITECT_EXTENSIONS` (`<data dir>/extensions/<id>/extension.toml`, as
   an install writes them; `features/sites/src/installed.rs`
   `InstalledBlocks`). An `[[embeds]] page` draws its fence with its page
   (`PageBlocks`); a WASI component's `[[embeds]] view` is mounted once
   per block in the extensions host's sandbox, its memory and time
   limited, and given the block as its input
   (`features/sites/src/component_blocks.rs` `ComponentBlocks`), so a
   renderer written in Rust runs on the node without being linked into
   it. The folder is read again when a package is added, removed or its
   manifest written; the next build draws with it.
2. **The extensions compiled into the build**: each linked extension's
   `[[embeds]] page` and the distribution's own renderers
   (`apps/distribution/src/lib.rs` `Distribution::blocks`, given to
   `apps/desktop/src/node/mod.rs` `start_with`). An embedder that starts
   the node from Rust hands it its own `architect_sites::Blocks`
   (`FnBlock`). A build of your own with Studio's charts compiled in
   draws a `kf` fence as the chart
   ([your own Architect](@/guide/your-own-architect.md)).

A block is drawn when the page is built, so it is rebuilt live with the
rest of the page, and only into the pages an audience can see: a note
outside a link's scope or another audience's filter never has its block
drawn there. What a block draws is made safe before it is written
(`features/sites/src/sanitize.rs`, by allow-lists): an inline SVG keeps
its drawing, classes and `data-*` (the player's `data-follow-playhead` /
`data-at`), HTML keeps text, lists, tables, links, images and media,
and URLs are only relative, `#…`, `https:` (links and media) or
`mailto:`, and an image only relative or a raster `data:` image;
scripts, handlers, forms, SMIL animation, `<foreignObject>`, and CSS with
a string, comment, escape, `var()`, custom property, at-rule, anything
that loads, or a `position` other than `static`/`relative` are dropped.
Each block sits in a box the site draws (`contain`, `overflow`), and its
SVG sheet is scoped to that box ([Sites](@/features/sites.md) has the
lists). Every page of a Site the node serves, and every file of a share
link, carries a Content-Security-Policy
(`apps/desktop/src/node/shares.rs` `content_security_policy`): scripts
from the node and the site's theme script by hash only, images also from
`https:` and `data:`, forms posted only to the node, no plugins or
`<base>`. WASI components are drawn by a node built with
feature `node-wasi` (the release's node and its image are); without it,
only page embeds and compiled-in renderers draw. Tests:
`apps/desktop/tests/shares.rs`
`a_claimed_fence_is_the_installed_extensions_drawing_live_and_per_audience`
and `an_embedders_own_blocks_draw_on_a_link_after_the_installed_ones`.

**Kept as org data.** A share is a note of the vault it shares,
`_architect/shares/<code>.md` (`share_note_path`), with `publish: false`, so
no Site ever shows it. It syncs to every device that holds the vault like
any other note. The node that serves the link reads every vault's shares
when it starts and again whenever the vault changes. A share made or
revoked on another device reaches it with the note, and one made from the
app over the org lane is held at once. Comments go in a note beside it,
`_architect/shares/<code>.comments.md`.

**Made from the app.** The node that hosts the org (`ARCHITECT_HOST_ORG`)
serves `architect_spaces::ShareLinks` on its org lane (`/org/<slug>/vox`,
the issuer session as the bearer): `create`, `shares`, `update`, `revoke`,
`add_audience`, `remove_audience` and `comments`. Any member lists shares. A member or above makes, changes and
revokes them (`SHARE_PERMITS`). The app's `SHARE_*` and `PUBLISH_DOMAIN`
actions call it (`features/accounts/src/share_actions.rs` `register`), and so
does its Share sheet, which `SHARE_SHEET` opens. The Organization menu has
"Share links…"; run from a file's context menu, `SHARE_SHEET` shares that
note or folder. The sheet
picks the audience, access and expiry, makes the link and shows it to copy,
and lists the org's shares, each with Revoke.

```sh
architect-desktop actions run SHARE_CREATE --arg folder=Album --arg audience=org --arg expires=7d --arg host=https://architect.fasttrackstudio.app
architect-desktop actions run SHARE_LIST
architect-desktop actions run SHARE_REVOKE --arg code=<code>
```

**Served** (`apps/desktop/src/node/shares.rs` `gate`, the node's outermost
layer, ahead of a Site mounted at `/`):

- Each live share is built as a scoped Site (`architect_sites::Scope`) at
  `/s/<code>/` into `<data>/shares/<code>/`: what its guests read (each
  page's plain blocks, `_note.json` beside the served folder) and the
  files it serves; its HTML pages are never served on the link. It is
  rebuilt whenever its vault changes: an edit on disk, from a peer's
  live bridge (#412) or from a web client on the org lane. The web app
  reads the link again on `/s/<code>/_live`, an event stream that fires
  after each rebuild. A revoke fires it as well, and the link reads as
  gone.
- **Nothing outside the scope is in it, so nothing outside it can be
  served.** A link to a note outside the scope is plain text with no URL. An
  embed of one is left out. No backlink, base row, search entry or tag page
  lists one. Under a folder scope only the folder's own attachments are
  copied ([sites](@/features/sites.md)). A path that is not a built page or
  a file of the link is `404`, and so are any `..` and a page the build
  wrote asked for by its file (`index.html`, `404.html`).
- **The audience is checked on every request.** `org` and `invited` need a
  person signed in at the issuer: their session as `Authorization: Bearer`
  (the #413 bearer), or the `architect_session` cookie. The node resolves it
  with the issuer's `whoami` and `orgs`, and caches the answer for a minute.
  With nobody signed in a page of the link is the app's page with `401`
  (the app offers the sign-in), a file plain `401`, and a person it was not
  shared with gets `403`. An expired, revoked or unknown code is
  `404`, and so is anything else under `/s/` that is not a live link.
- **Signing in** (`/s/_signin`) is the issuer's device sign-in. The node
  starts one and links to its approval page, the person approves it there
  (signed in at the issuer), and the node sets the session as an `HttpOnly`,
  `SameSite=Lax` cookie, `Secure` when the request came over `https`. The
  node reads that from `X-Forwarded-Proto`, and only when a request has
  none from its issuer's scheme (an `https` issuer: `Secure`). **The proxy
  in front of a production node must send `X-Forwarded-Proto: https`**: a
  proxy that sends `http`, or none in front of a node whose issuer is
  `http`, gets a cookie without `Secure`. The node never sees a password. Signing out is a form on `/s/_signout` (a GET shows it): its
  POST, from the node's own page (its `Origin` checked), clears the cookie.
- **A link's own files never run on the node's origin.** It is the web
  app's origin, so a page, an SVG, an XML document or a script a note
  links (anything but a picture, a recording, a video, plain text or a
  PDF) is answered as a download (`Content-Disposition: attachment`)
  under `Content-Security-Policy: sandbox; default-src 'none'` and
  `nosniff`: opened, it runs nothing and reads nothing of the app's
  session (`apps/desktop/src/node/shares.rs` `inert`). A share with a
  domain of its own is its own origin and serves its pages as a Site.
- **Answers keep the code private:** `Referrer-Policy: no-referrer` on every
  response; `X-Robots-Tag: noindex` unless the share is public; `private,
  no-store` for a signed-in audience.
- **Comments.** The node-rendered page's comment form went with the page
  (#646): a link takes no comment from a form. A share's comments stay in
  their note and its owners list them (`ShareLinks::comments`); a guest's
  comments come back as a method of the guest service.

`apps/desktop/tests/shares.rs` tests all of it against the in-process issuer:
the scope never leaks, each audience, expiry and revoke, a live edit read by
the link's guest, a domain, named audiences, the files and the archive.

### One share, many audiences

A setlist goes to a whole band, and each player needs a different slice of
it: the violins their charts, the leader everything (#510). A share carries
**named audiences** (`features/spaces/src/share.rs` `ShareAudience`), each a
filter over the share's scope:

| | |
| --- | --- |
| **Name** | `violin`, `rhythm`: once per share, any case |
| **Filter** | a Bases filter expression over a note's properties and file, the language of a `.base` file's `filters:` (`instrument == "violin"`, `instrument.contains("violin")`, `file.hasTag("rhythm")`); empty holds for every note. The Bases engine evaluates it (`features/sites/src/listing.rs` `parse_filter`, `select`); there is no second filter language |
| **Link** | its own code: `https://<node>/s/<the audience's code>/`. Knowing one audience's link tells nothing about another's, or about the share's own |

```sh
architect-desktop actions run SHARE_AUDIENCE_ADD --arg code=<code> --arg name=violin --arg 'filter=instrument == "violin"'
architect-desktop actions run SHARE_AUDIENCES --arg code=<code>      # name, filter, code, url of each
architect-desktop actions run SHARE_AUDIENCE_REMOVE --arg code=<code> --arg name=violin
```

- **The share's own link stays the whole scope** (the leader's link).
  *Who* may open any of its links is still the share's `Audience` (public,
  link, org, invited), its expiry and its revoke: an audience here is which
  part a link shows, not who is let in.
- **Nothing else is listed.** Each audience is built as its own site, of
  the notes its filter holds for alone (`architect_sites::Options::filter`),
  so no page, nav entry, backlink, base row, tag page or search entry names
  a note outside it.
- **Nothing else is served, checked when the request comes.** The node
  reads the vault again whenever it has changed, and answers a request to
  an audience's link only if the note asked for is in the scope and the
  filter holds for it now (`apps/desktop/src/node/shares.rs` `Guard`). A
  file is served only if a note the filter holds for attaches it. Another
  audience's page or file is `404` there, on the page route and the byte
  route alike, whatever is on disk. A note whose property changes leaves
  the link with that change.
- **Removed alone.** `SHARE_AUDIENCE_REMOVE` takes one audience's link
  down (`404` from the next request); the share and its other audiences
  stay. Revoking the share takes every link with it.
- **Kept in the share's note.** Audiences are part of the share
  (`Share::audiences`), so they sync with it. A share note written before
  audiences reads as a share with none.

A filter is over **notes**: attachments carry no properties of their own
on today's vault, so a file belongs to the notes that link or embed it. Give
each part its own note (`instrument: violin`, `song: Joy to the World`, the
PDF embedded) and a filter on the note is a filter on its file.

### Files and Download all

Every link has its files in the web app: `<link>_files/` opens the app on
the link's Files view (`apps/web/src/app/guest.rs` `FilesView`), each file
a download, grouped by note in list order, and "Download all". What it
lists is the guest service's `files` (the same as `<link>_files.json`):
what the link shows, read from the vault when it is asked for, with an
audience's filter and the link's layer applied. The archive takes a view
(`?by=`):

| View | `by` | In the archive |
| --- | --- | --- |
| **In order**: the list as it is ordered | `order` | `01 Violin I.pdf`, numbered file by file |
| **By note**: a group per note | `note` | `01 Joy to the World/Violin I.pdf`: a folder per note, numbered by its place in the list |
| **By a property**: a group per value (a note with several values is in each) | `property:<key>` | `violin/01 Joy to the World/Violin I.pdf`: a folder per value, its notes numbered in it |

List order is the scope's: a note list's own order, a base's rows in its
views' order, otherwise by path (`architect_sites::Scope::ordered`).

**Download all** in the app is `<link>_archive.zip?by=note`; any view or
group is `<link>_archive.zip?by=<view>[&group=<label>]`: a zip of the files in view,
in the folders above. The node writes it to the response while it reads the
files (`features/sites/src/archive.rs` `ZipWriter`): it is never whole in
memory and never on disk, files are stored as they are, and it carries no
`Content-Length` (and takes no ranges). An audience's archive holds exactly
its files. From the app or an agent:

```sh
architect-desktop actions run SHARE_ARCHIVE --arg code=<code> --arg audience=violin --arg to=~/Downloads/violins.zip
architect-desktop actions run SHARE_ARCHIVE --arg code=<code> --arg by=property:song --arg 'group=Joy to the World' --arg to=/tmp/joy.zip
```

### A link that plays

A setlist shared as a link plays in the browser, with nothing to install
(#511, #646). Give each song its own note, its recording embedded
(`![[media/joy.mp3]]`) and its synced lyrics beside it (`![[media/joy.lrc]]`,
or an `lrc` fence in the note), and its art as its `art:` property
(`art: "[[cover.jpg]]"`):

- **The link's recordings on the Now Playing bar.** The web app queues
  every audio and video file of the notes the link lists, in list order
  (`apps/web/src/guest_view.rs` `queue`), as its own source on the
  playhead, with each note's art (`ListedNote::art`, only when the link
  serves it). A row's play button plays its song; the bar's card shows
  the song and its art and opens the now-playing view. The app's one
  `<audio>` element plays a file from the link's own URL
  (`apps/web/src/app/guest.rs` `file_url`), so there is no page load
  between songs.
- **Streamed with ranges.** The node serves every file of a link with HTTP
  ranges: `206` with `Content-Range` and `Accept-Ranges: bytes`, `416` past
  the end (`architect::http::ranged::serve_file`), so the player seeks
  without fetching the file. A range is a request like any other: the
  audience and the audience's filter are checked first, and the other
  audience's file is `404` on a link, ranged or not.
- **Lyrics and charts.** A note's synced lyrics reach a guest as its
  words (the build's lines); a chart a fence draws reaches it as a drawn
  block's words and claim. Drawing them so they follow playback is the
  view of the extension that claims the note's type, handed the note's
  text and the link's art and lyrics when it declares `reads`
  ([what a guest's view reads](#a-link-opens-architect)); for Studio's
  charts and lyrics that is Studio's island.

Not yet: streaming from `architect-files` and its renditions (a small proxy
before the full file, #490; today the files are the vault's own, copied into
the link).

Tested on a headless node (`apps/desktop/tests/shares.rs`
`media_streams_with_ranges_lyrics_follow_and_the_queue_is_the_links_order`)
and in a headless Firefox (`apps/web/test/app-share-guest.test.mjs`, `just
app-share-guest-test`): the link's notes in their sets with their art and
play buttons, a note and its picture, the files and their archive; a link
for signed-in people opened through `/s/_signin` and the node's cookie,
past a session the issuer no longer knows; a node with no web app.

### How far a link plays

A song can go out at three depths (#528), and the sharer says how deep each
link goes (`features/spaces/src/share.rs` `Listen`):

| Layer | `listen` | What the link serves |
| --- | --- | --- |
| **Page** | `page` | the pages, and every file no extension places higher (charts, lyrics, downloads) |
| **Reference** | `reference` | also what a player needs for the song's mix with a live click and guide, the project shown read-only (Studio's Reference Mode) |
| **Full** | `full` | every file: the stems, for the full multitrack engine |

- **The sharer sets the most.** `listen` is on the share (its own link and
  its domain) and on each audience, which only narrows it: an audience set
  higher than its share plays to the share's
  (`features/spaces/src/share.rs` `Share::listen_for`). A new link plays to **Reference** unless told
  otherwise. The orchestra's audience gets Reference, the band's Full.
- **The viewer opts in within it.** A page plays what is on it; a player
  that loads deeper (Studio's "Load multitracks") offers it only when the
  link allows.
- **The node holds it, per file.** Architect does not know a song's folder
  layout: the extension that owns the note type places its files with
  `[[share_layers]]` rules ([share layers](@/architecture/extensions.md#share-layers)).
  A file above the link's layer is `403` with the layer it needs in the
  body and in `x-architect-layer`, by its name and by any other spelling of
  the same file on disk (another case, another Unicode form, `//`): the
  node compares what the request opens with the files it holds back
  (`apps/desktop/src/node/shares.rs` `Shares::above`). Only a file in the
  link's own folder is answered so: anything outside the link, another
  audience's file, a page it does not have, is the same `404` as a path
  that is nowhere (status, headers and body), so a link says nothing of
  what is outside it. Ranges keep working for the rest. A file no rule
  places is a page file.
- **Named nowhere, never on disk.** Each link is built for its layer: a
  held-back file is not copied into its folder at all, and is in no page of
  it (no player, link, embed or lyrics), its search index, its files listing,
  its archive or its queue. The node answers `403` for it from where files
  are placed, by its path, before it looks at the disk; a folder being
  swapped in by a rebuild is waited for first, and one it cannot read is
  `503`, never served. A folder built before a change of layer is checked
  by what the request opens on disk; a file there that the link no longer
  lists but that is now above its layer answers the same `404` as a path
  that is nowhere.
- **Unknown is closed.** Where files are must be known from a vault read
  whole. While any part of the vault does not read (a note that is not
  UTF-8, a file without permission, one half-written by a sync, a folder
  the walk cannot enter) or where files are is otherwise unknown (below), a
  link below Full answers `503` to everything, its pages included, and is
  not built again until it reads; its pages come back with the vault.
- **Closing is at once; opening a moment later.** A layer lowered through
  `SHARE_UPDATE` or the Share sheet holds files back from the next request.
  A file that opens (a layer raised, a rule lowered or forgotten) is served
  once the link is built again, a moment later. A change in the vault (a
  note's `type` becoming `song`) reaches the node about half a second after
  the write: until then the file is placed as it was. A download of the
  archive already under way is asked again before each file and stops at
  the first one its link no longer serves (revoked, expired, its layer
  lowered): the zip then does not open.
- **Nothing an extension does opens a link.** Every rule for a note type
  counts and the highest layer wins. A development copy (installed from a
  path, its slot holding only `installed.toml`) is read where it links to.
  While an installed extension's manifest does not read (half-written, not
  a manifest, an unknown key in `[[share_layers]]`, a link to nothing) or
  two folders claim one id, the node does not know where files are and
  holds every attached file back on every link below Full until it is
  fixed, warning once with the file's path. An uninstalled extension's
  rules are kept, by its folder, in the node's data dir
  (`share-layers.json`) and still place files until the owner forgets
  them. If that file does not read, it is set aside
  (`share-layers.json.corrupt-<time>`), never written over, and every
  attached file is held back below Full, across restarts, until the owner
  acknowledges it; if it cannot be written, the same (with what could not
  be written left beside it when the folder takes it). A reinstall that declares fewer rules lowers files as
  it says, and the node warns, listing what was lowered, until
  acknowledged. The node's status (`NodeStatus::share_layers`) says when
  any of these needs the owner:

  ```sh
  architect-desktop actions run SHARE_LAYERS_KEPT                       # kept rules and notices
  architect-desktop actions run SHARE_LAYERS_FORGET --arg extension=fts.studio
  architect-desktop actions run SHARE_LAYERS_ACKNOWLEDGE                # a set-aside file, lowered rules
  ```
- **A share note without a layer** (written before layers, or one that
  lost the field) reads as Reference, the default, and its audiences as
  the share's: access fails closed. A hand-edited `"listen"` reads in any
  case (`full`); a value that is no layer leaves the note unread (`404`). Anyone who can write the vault can
  edit a share note, so a cap is as strong as the vault's write access
  (#552).

```sh
architect-desktop actions run SHARE_CREATE --arg folder=Songs --arg listen=reference
architect-desktop actions run SHARE_UPDATE --arg code=<code> --arg listen=full
architect-desktop actions run SHARE_AUDIENCE_ADD --arg code=<code> --arg name=orchestra --arg listen=reference
architect-desktop actions run SHARE_AUDIENCE_UPDATE --arg code=<code> --arg name=orchestra --arg listen=share   # back to the share's
```

The Share sheet shows the same as one choice per link and per audience.

**The listing, for a client.** `<link>_files.json` is what a link serves,
for a program that opens a song over it (Studio's `SongSource` over an
Architect link):

```json
{
  "link": "/s/<code>/", "title": "Songs",
  "listen": "reference", "max": "full",
  "notes": [{ "path": "Songs/Joy/Joy.md", "title": "Joy", "type": "song", "page": "/s/<code>/joy/joy/", "files": ["…"] }],
  "files": [{ "path": "Songs/Joy/Joy.session", "url": "/s/<code>/Songs/Joy/Joy.session",
              "size": 18211, "sha256": "…", "layer": "reference", "note": "Songs/Joy/Joy.md" }]
}
```

`max` is the most the link plays; `listen` is the layer listed, the link's
unless `?listen=` asks for less (never more). Each file is listed once, in
list order, with its vault's size and sha256 (`null` until the vault has
hashed it). Tested on a headless node (`apps/desktop/tests/shares.rs`
`a_links_layer_is_held_by_the_node_on_every_route`,
`a_share_written_before_layers_serves_what_it_served`,
`the_apps_share_actions_set_and_read_how_far_a_link_plays`).

## Its own deployment

A share, or a whole vault, can also live at a URL of its own. There are two
ways, simplest first:

1. **A domain mapped to a share.** Give the share a domain (`SHARE_UPDATE`
   with `domain`, or `PUBLISH_DOMAIN`). The node then serves that share at
   the domain's root as well, built with base `/`. Point the domain's DNS at
   the node's ingress and add the host there with a certificate. Nothing
   else changes: same node, same audience, expiry and revoke, same live
   rebuilds.
2. **A static export.** Build the same scoped site into a folder, and host
   it anywhere static files are served:

   ```sh
   architect site build --vault ~/notes --folder Album --base / --url https://album.example.com --out dist
   # or --tag, --note (repeatable), --base-scope; `just docs` builds this repo's docs the same way
   ```

   `PUBLISH_EXPORT` does the same from the app. An export is a snapshot: it
   has no audience, expiry or revoke, and no live updates.

A separate headless instance (another `serve` with its own `ARCHITECT_SITES`)
also works. It is one more node to run, so use it only when a site must be
isolated from the org's node.

## The image

`flake.nix` builds it with `dockerTools.streamLayeredImage`, the way Task's
images are built (no Docker). `architect-image` compiles the node in the Nix
sandbox. `architect-image-prebuilt` wraps a binary built in the dev shell
(`ARCHITECT_PREBUILT_BIN`, `nix build --impure`). The image runs
`architect-desktop serve` as its entrypoint, exposes `8080/tcp` (HTTP) and
`7777/udp` (iroh), and keeps its state in the `/data` volume.
Another distribution's image is the same, with its binary and its name
(`ARCHITECT_IMAGE_BIN`, `ARCHITECT_IMAGE_NAME`); `cargo xtask dist
node-image` builds it ([your own Architect](@/guide/your-own-architect.md)).

```sh
nix build .#architect-image
./result | nix run nixpkgs#skopeo -- copy docker-archive:/dev/stdin docker-daemon:architect:dev
```

A release tag `vX.Y.Z` publishes it (`.github/workflows/release.yml`, job
`node`) as `ghcr.io/fasttrackstudios/architect:X.Y.Z`, `:sha-<short>` and
(for the highest release) `:latest`, and attaches the binary to the release.

## Self-hosting with the chart

`deploy/chart/architect` runs Architect on Kubernetes: the headless node
and the web front end ([the web front end](@/guide/web.md)) as two
Deployments with two Services, each with its own `replicaCount`, on one
host. starcommand runs architect.fasttrackstudio.app from this chart.

```sh
kubectl create secret generic architect-node --from-file=session=./session.token
helm install architect deploy/chart/architect \
  --set host=architect.example.com \
  --set node.issuer=https://auth.example.com \
  --set node.hostOrg=my-org \
  --set node.existingSecret=architect-node \
  --set ingress.enabled=true --set ingress.className=nginx
```

| Values | What |
| --- | --- |
| `host` | the hostname both sides share |
| `web.replicaCount`, `web.config.issuer`, `web.config.clientId` | the static front end; its `/app/config.json` comes from a ConfigMap (the issuer defaults to `node.issuer`) |
| `node.issuer`, `node.org`, `node.hostOrg`, `node.vaults`, `node.sites`, `node.env` | the node's settings ([Configure it](#configure-it)) |
| `node.existingSecret` | a Secret mounted as files: `session` (`ARCHITECT_SESSION_FILE`) and, with `node.mcp`, `mcp-token` |
| `node.persistence` | the data PVC at `/data` (its iroh key and documents). A node is one device, so `node.replicaCount` above 1 needs persistence off, and the chart refuses otherwise. With it off, `/data` is an `emptyDir` of at most `emptyDirSizeLimit` (10Gi) |
| `node.store` | the blob store's budget: `max` (`ARCHITECT_FILES_STORE_MAX`; empty is 75% of the data volume; a size such as `8Gi` or `1.5Gi`, checked against the volume), `keepFree` (`ARCHITECT_FILES_KEEP_FREE`, `15%`) and `historyMax` (`ARCHITECT_FILES_HISTORY_MAX`, empty keeps every replaced version) |
| `node.ephemeralStorage` | the container's own `ephemeral-storage` (2Gi): its writable layer and logs. With persistence off the `emptyDir` `/data` counts too, so the limit set is this plus `emptyDirSizeLimit`. Past it the kubelet evicts this pod, not the host's others |
| `node.files` | the files dir at `/files` (`ARCHITECT_FILES_DIR`, held in place): `nfs: {server, path}`, `hostPath` or `existingClaim` |
| `node.s3` | the files' bytes in a bucket too (`ARCHITECT_BLOB_S3_*`). The key id and the secret come from `credentialsSecret`, never from values |
| `node.s3Face` | the node's own S3 face (`ARCHITECT_S3_BIND`): a Service port and, with `host`, a host of its own on the Ingress (buckets are at its root, so it can't share the path space) |
| `node.iroh` | UDP 7777: `service: LoadBalancer` (or `NodePort`), or `hostPort: true`. Without either, peers reach the node through n0's relays |
| `ingress` | the optional Ingress with the path split below |

Probes are `GET /health` on the node and `GET /` on the web front end.

**What bounds the node's disk.** A PVC's size is a request: the `local-path`
provisioner (k3s's default) and `hostPath` volumes are plain directories on
the host and enforce nothing, and the kubelet's `ephemeral-storage` limits do
not count a PVC. What bounds the node's writes to `/data` there is the node
itself: `node.store.max` and `node.store.keepFree` (the keep-free floor
default, 15%, is above the kubelet's 10% disk-pressure threshold, so the node
stops writing before the host is tainted). The kubelet does evict this pod for
its writable layer, its logs and an `emptyDir` `/data` past their limits. A
holding node's tree-side areas live on the files volume (NFS), under the same
15% floor of that volume and `node.store.historyMax`. A
storage class that enforces quotas (a block device per claim, or a filesystem
with project quotas) bounds the PVC as well.

### Routing

One host, two backends. The node owns these prefixes (`node.paths`, plus
`node.extraPaths` for each Site mount), and everything else goes to the
web front end:

| Prefix | On the node |
| --- | --- |
| `/vox` | vox over WebSocket |
| `/org` | `/org/<slug>/vox`, the org lane (vaults, share links) |
| `/mcp` | MCP, with its bearer |
| `/health` | liveness and readiness |
| `/s` | share links: `/s/<code>/…` (the web app's page for a page path, the link's files, `_files.json`, `_archive.zip`, `_live`, its guest service `_guest/`) and their sign-in `/s/_signin` |
| `/node` | the node's HTTP face (`POST /node/status`, which the web app reads) |
| `/.well-known` | `apple-app-site-association`, when `ARCHITECT_APPLE_APP_IDS` is set |
| `/blobs` | reserved for the node |

The chart's Ingress writes exactly this, with `X-Forwarded-Proto` left to
the controller (the share sign-in cookie is `Secure` only when it says
`https`: a proxy of your own must send it too).
`deploy/chart/architect/tests/routes.sh` renders it and checks that each
prefix reaches the node and `/` the web front end (`pr.yml` runs it with
`helm lint`). With a proxy of your own (Caddy, Traefik, nginx), route the
same prefixes. The web front end has no single-page fallback, so a prefix
you forget answers `404` instead of the app's page. Keep the
`Sec-WebSocket-Protocol` request header out of the proxy's access log: a
browser's bearer travels in it on `/org/<slug>/vox` and `/vox`. The node
opens its org lane to a browser on its own origin only (the request's
`Host`, or `X-Forwarded-Host`): a proxy that rewrites both, or a web app on
another origin, needs that origin in `ARCHITECT_LANE_ORIGINS`.

A share link's page (`/s/<code>/`) is the node's copy of the web app's
page, which loads the app from `/app/` on the same host: the web front
end's. The chart tells the node so (`ARCHITECT_WEB_APP=beside`) whenever
`web.enabled` is on. With `web.enabled=false`, a node of your own, the
desktop's or Studio's, nothing serves `/app/`: leave `ARCHITECT_WEB_APP`
unset (`node.webApp` in the chart) and a link's page says it opens in the
web app, which this server does not serve, rather than waiting for a
script that is not there; set it only when you serve the same release's
web bundle at `/app/` beside the node. The page carries the node's share
contract and version (`<meta name="architect-share-shell">`,
`architect-node-version`): an app of another release says it is out of
date, and a bundle too old to know share links leaves a line saying the
two must be the same release.

## Without Kubernetes: systemd

A server without Kubernetes runs the release binary. Each release attaches
`architect-node-<tag>-x86_64-linux.tar.gz` (and its `.sha256`), the node
built without the window:

```sh
v=v0.11.0   # the release
curl -fLO https://github.com/FastTrackStudios/architect/releases/download/$v/architect-node-$v-x86_64-linux.tar.gz
curl -fLO https://github.com/FastTrackStudios/architect/releases/download/$v/architect-node-$v-x86_64-linux.tar.gz.sha256
sha256sum -c architect-node-$v-x86_64-linux.tar.gz.sha256
tar xzf architect-node-$v-x86_64-linux.tar.gz
sudo install -m755 architect-node-$v-x86_64-linux/architect-desktop /usr/local/bin/
sudo install -Dm644 deploy/systemd/architect-node.service /etc/systemd/system/architect-node.service
sudo install -Dm600 session.token /etc/architect/session
printf 'ARCHITECT_NAME=home\nARCHITECT_ISSUER=https://auth.example.com\nARCHITECT_HOST_ORG=my-org\n' | sudo tee /etc/architect/node.env
sudo systemctl enable --now architect-node
```

`deploy/systemd/architect-node.service` runs it as a dynamic user with
its state in `/var/lib/architect`, HTTP on `127.0.0.1:8080` (put a reverse
proxy with TLS in front, routing the prefixes above) and iroh on UDP 7777.
The session is a systemd credential, read as a file. To serve the web
front end too, unpack `architect-web-<X.Y.Z>.tar.gz` and serve it with
any static server, as the proxy's default backend.

## On starcommand

starcommand deploys architect.fasttrackstudio.app from this chart: an Argo
CD Application tracking the highest release tag, argocd-image-updater
rolling both images (`semver` on `^[0-9]+\.[0-9]+\.[0-9]+$`, `helm.image-name`
per image), and Caddy routing the node's prefixes above (its
`modules/services/architect`). The images are on GHCR, made public once in
the org's package settings.

## Not yet

- **Share sign-in by redirect.** A signed-in audience signs in by approving a
  device code at the issuer. An OAuth redirect back to the link would be one
  click fewer.
- **A share is served by the node that hosts the org.** Any node that holds
  the vault knows its shares and would serve them, but the app makes them on
  the host (`host`).

- **An org's enrolments live on its host.** The issuer keeps the account's
  devices but not an org's: two nodes hosting one org hold two tables, and the
  app is told which node to ask (`host`).
- **The web app's side.** The org lane serves signed-in clients, but the
  Architect web app does not open it yet: it needs the node's address for the
  org (as `ORG_*` take `host`) and a `SyncedDoc` per open note. The root `/vox`
  stays the public `Node` service only.
- **Live bridges per note, not per vault.** A node keeps a session per note
  with each peer, up to `ARCHITECT_LIVE_MAX`; a large vault's other notes wait
  for the sweep. One multiplexed session per vault would lift the cap.
- **A second seed's own text.** When two seeds of a note differ, the second's
  text is in the history but not the note. A three-way merge needs a base the
  two copies never shared.
