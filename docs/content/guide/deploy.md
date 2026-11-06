+++
title = "Deploy: Architect, headless"
description = "architect-desktop serve: the Architect app run with no window as an always-on node, a sync peer for the org's vaults that serves Sites, MCP and the HTTP face, released as a pure-Nix OCI image."
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
- `ARCHITECT_NAME`, `ARCHITECT_ISSUER`, `ARCHITECT_ORG`, `ARCHITECT_VAULTS`
  and `ARCHITECT_SITES` as values, and the session and MCP bearer as a
  Secret mounted as files (`ARCHITECT_SESSION_FILE`,
  `ARCHITECT_MCP_TOKEN_FILE`).

## Not yet

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
