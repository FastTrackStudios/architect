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
| **Its lanes over iroh** | The node's documents (`crdt::p2p::node_router`: `DocSync` and `DocPeer`), served to the devices its `Admission` lets in. A revoked device's connections close (`architect_devices::serve`). |
| **A sync peer for the vaults** | Each markdown file of each vault is a document (`vault_collab::VaultCollab`, written behind to the file; an edit on disk is merged in). `apps/desktop/src/node/catalog.rs` `VaultCatalog` names them on the peer lane. Every `ARCHITECT_SWEEP_SECS` the node pulls from and pushes to each peer it can dial (`crdt::peer::pull`), so an edit on one device reaches another through it. |
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
| `ARCHITECT_PEERS` | none | `endpoint@host:port,…`: nodes it dials directly |
| `ARCHITECT_IROH` | `n0` | `n0` (relays and lookup) or `local` (loopback only, for tests) |
| `ARCHITECT_IROH_BIND` | any port (`0.0.0.0:7777` in the image) | the iroh UDP socket |
| `ARCHITECT_MCP_TOKEN_FILE` | none | `/mcp` is served only with this bearer |
| `ARCHITECT_SWEEP_SECS` | `30` | how often it exchanges documents with its peers |

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

- **Org enrolment.** The node joins the account (`Grant::Account`). It does
  not yet serve `architect_devices::server::EnrolmentHost` on org lanes, so
  another member's devices cannot be enrolled with it (`Grant::Org`).
- **Signed-in web clients on `/vox`.** The vox and HTTP lanes carry only the
  public `Node` service. The vaults reach a browser through a Site, not live.
  A web app editing a vault needs an issuer-bearer identity on those lanes.
- **Live sessions.** Peers converge by sweeps (`ARCHITECT_SWEEP_SECS`), not by
  a live `DocSync` session per open note.
- **A vault's first copy.** Two nodes that each made "the same" note on their
  own hold two documents of one id (`crdt::peer::FOREIGN_LINEAGE`). Seed a
  vault on one node and let the others pull it.
