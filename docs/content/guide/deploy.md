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
cargo run -p architect-desktop --no-default-features --features vox -- serve
```

`serve` is a run mode of the app's own binary (`apps/desktop/src/main.rs`),
beside `actions`, `mcp` and `--headless`, so it is one product with one
binary and one version. Built with `--no-default-features --features vox`
(no `gui` feature), the binary is the node alone: no Blitz, wgpu or wayland,
and no GPU libraries in the image. The desktop build is unchanged.

## What it does

`apps/desktop/src/node/mod.rs` `start`:

| | |
| --- | --- |
| **On the account** | With an issuer and a session it registers itself in the account's address book as a `server` device at its iroh endpoint, offers its org (`ARCHITECT_ORG`) if the account shares none yet, and lets the account's other devices in (`architect_devices::account::pass`). It runs again every five minutes, and on every sweep until another device is on the account. |
| **In an issuer org** | With `ARCHITECT_HOST_ORG` it hosts the org's enrolments and its vaults for signed-in web clients (below); with `ARCHITECT_ENROL_AT` it enrols itself in an org another node hosts. Its vaults are that org's. |
| **Its lanes over iroh** | The node's documents (`crdt::p2p::node_router`: `DocSync` and `DocPeer`), served to the devices its `Admission` lets in. A revoked device's connections close (`architect_devices::serve`). |
| **A sync peer for the vaults** | Each markdown file of each vault is a document (`vault_collab::VaultCollab`, written behind to the file; an edit on disk is merged in). `apps/desktop/src/node/catalog.rs` `VaultCatalog` names them on the peer lane. With each peer it dials, the node keeps a live `DocSync` bridge per note (below), so an edit reaches the others within a second. Every `ARCHITECT_SWEEP_SECS` it also sweeps each peer (`crdt::peer::pull_with`), the fallback for what a bridge missed. |
| **Sites** | Each vault in `ARCHITECT_SITES` is built as a Site ([sites](@/features/sites.md)) and rebuilt when the vault changes. |
| **Share links** | `/<code>/`: a share's scope of a vault as a Site, to the audience it names, until it expires or is revoked ([below](#share-links)). |
| **The HTTP face and vox** | `apps/desktop/src/node/service.rs` `Node` (`status`) over same-origin vox at `/vox`, its HTTP face (`POST /node/status`) and the Action List (`NODE_STATUS`). |
| **MCP** | `/mcp`, only when `ARCHITECT_MCP_TOKEN_FILE` holds a bearer: `NODE_STATUS` and the `NOTES_LIST`, `NOTES_READ` and `NOTES_WRITE` actions (`apps/desktop/src/node/service.rs` `Notes`). An agent writes a note on the always-on node, and every device that syncs with it gets the note. |
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
| `ARCHITECT_ISSUER` | none | the account server, `https://auth.fasttrackstudio.app` |
| `ARCHITECT_SESSION_FILE` | none | a file holding the account's session token |
| `ARCHITECT_ORG` | none | the org it offers the account while the account shares none |
| `ARCHITECT_HOST_ORG` | none | an issuer org (slug) the session is in: the node serves its enrolment lane at `/org/{slug}/vox` |
| `ARCHITECT_ENROL_AT` | none | another node's HTTP origin: this node enrols itself there, once |
| `ARCHITECT_ENROL_ORG` | none | the issuer org (slug) it enrols in at `ARCHITECT_ENROL_AT` |
| `ARCHITECT_ENROL_ROLE` | `member` | the role it asks for, at most the session's own in the org |
| `ARCHITECT_PEERS` | none | `endpoint@host:port,…`: nodes it dials directly |
| `ARCHITECT_IROH` | `n0` | `n0` (relays and lookup) or `local` (loopback only, for tests) |
| `ARCHITECT_IROH_BIND` | any port (`0.0.0.0:7777` in the image) | the iroh UDP socket |
| `ARCHITECT_MCP_TOKEN_FILE` | none | `/mcp` is served only with this bearer |
| `ARCHITECT_SWEEP_SECS` | `30` | how often it sweeps its peers, the fallback for what a live bridge missed |
| `ARCHITECT_LIVE_MAX` | `256` | how many notes it keeps live with each peer; `0` sweeps only |

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
  `/org/<slug>/vox`. A caller presents an issuer session as its bearer: the node
  resolves it with the issuer's `whoami` (`apps/desktop/src/node/org.rs`
  `IssuerSessions`) and learns the person's orgs and role from the issuer
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

A share link puts part of a vault on `https://<node>/<code>/`, the way you
share a topic with someone (#374). The model is #197's share links
(`features/spaces/src/share.rs` `Share`):

| | |
| --- | --- |
| **Scope** | the whole vault, a folder (the link is rooted there), a tag, a list of notes, or the notes a `.base` file selects (`ShareScope`) |
| **Code** | 16 random characters of base32, 80 bits (`code_from`): the link's first segment |
| **Audience** | `public` (anyone, may be listed), `link` (anyone with the link, `noindex`), `org` (the org's members, signed in at the issuer), `invited:<emails>` (those people, signed in) (`Audience`) |
| **Access** | read-only, or allowed to comment (`ShareAccess`) |
| **Expiry** | an optional time, after which the link is `404` |
| **Revoke** | final; the link is `404` from the next request on, and its pages are deleted |
| **Domain** | optional: the same share at the root of a host of its own |

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
the issuer session as the bearer): `create`, `shares`, `update`, `revoke` and
`comments`. Any member lists shares. A member or above makes, changes and
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
  `/<code>/` into `<data>/shares/<code>/`. It is rebuilt whenever its vault
  changes: an edit on disk, from a peer's live bridge (#412) or from a web
  client on the org lane. An open page reloads itself through `/<code>/_live`,
  an event stream that fires after each rebuild. A revoke fires it as well,
  so the page reloads to `404`.
- **Nothing outside the scope is in it, so nothing outside it can be
  served.** A link to a note outside the scope is plain text with no URL. An
  embed of one is left out. No backlink, base row, search entry or tag page
  lists one. Under a folder scope only the folder's own attachments are
  copied ([sites](@/features/sites.md)). A path that is not a built page is
  `404`, and so is any `..`.
- **The audience is checked on every request.** `org` and `invited` need a
  person signed in at the issuer: their session as `Authorization: Bearer`
  (the #413 bearer), or the `architect_session` cookie. The node resolves it
  with the issuer's `whoami` and `orgs`, and caches the answer for a minute.
  With nobody signed in the link is `401` with a Sign in button, and a person
  it was not shared with gets `403`. An expired, revoked or unknown code is
  `404`.
- **Signing in** (`/_share/signin`) is the issuer's device sign-in. The node
  starts one and links to its approval page, the person approves it there
  (signed in at the issuer), and the node sets the session as an `HttpOnly`,
  `SameSite=Lax` cookie (`Secure` behind `https`). The node never sees a
  password. `/_share/signout` clears the cookie.
- **Answers keep the code private:** `Referrer-Policy: no-referrer` on every
  response; `X-Robots-Tag: noindex` unless the share is public; `private,
  no-store` for a signed-in audience.
- **Comments.** A share that takes comments shows each page's comments and a
  form under it (`POST /<code>/_comment`). The comment is signed by the
  person if they are signed in, and by the name typed, marked "not signed
  in", if not. A read-only share refuses with `403`.

`apps/desktop/tests/shares.rs` tests all of it against the in-process issuer:
the scope never leaks, each audience, expiry and revoke, a live edit on the
shared page, comments, and a domain.

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

```sh
nix build .#architect-image
./result | nix run nixpkgs#skopeo -- copy docker-archive:/dev/stdin docker-daemon:architect:dev
```

A release tag `vX.Y.Z` publishes it (`.github/workflows/release.yml`, job
`node`) as `ghcr.io/fasttrackstudios/architect:X.Y.Z`, `:sha-<short>` and
(for the highest release) `:latest`, and attaches the binary to the release.

## Deploying it

ArgoCD in starcommand tracks the latest release image, as it does for Task
(argocd-image-updater, `semver` strategy on `^[0-9]+\.[0-9]+\.[0-9]+$`). The
pod needs:

- the image `ghcr.io/fasttrackstudios/architect`, its tag rolled by the image
  updater;
- port `8080` (HTTP) behind the ingress, and `7777/udp` if peers should reach
  it directly rather than through an iroh relay;
- a persistent volume at `/data`: its iroh key (a stable endpoint id) and its
  documents;
- liveness and readiness on `GET /health` (port `8080`);
- every path of its host routed to it (share links are `/<code>/` at the
  root, beside a Site at `/`), with `X-Forwarded-Proto` set so the sign-in
  cookie is `Secure`; and, for each share with a domain of its own, that
  host on the same ingress with a certificate;
- `ARCHITECT_HOST_ORG` for the org's lane, which the app makes shares on,
  and for the `org` audience;
- `ARCHITECT_NAME`, `ARCHITECT_ISSUER`, `ARCHITECT_ORG`, `ARCHITECT_VAULTS`
  and `ARCHITECT_SITES` as values, and the session and MCP bearer as a
  Secret mounted as files (`ARCHITECT_SESSION_FILE`,
  `ARCHITECT_MCP_TOKEN_FILE`).

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
