+++
title = "The whole stack on your machine"
description = "devenv up: the auth server, a headless node and the app, wired to each other and to nothing else; filled from a sanitised mirror of production on a credentialed machine and from the demo seed everywhere else."
weight = 74
+++

`devenv up` with no arguments starts everything Architect needs on one
machine: an auth server, a headless node signed in to it, and the app
signed in to both. Every process binds `127.0.0.1`, nothing names
production, and on a machine that holds an administrator's token the
accounts and orgs are a sanitised copy of production's, so you can test
with production's shape without touching it.

## Run it: `just dev`, `just demo`, `just website`

The same three recipes, with the same meaning, in Architect, Studio and
Keyflow (`scripts/run-it.sh` here):

| Recipe | What it runs |
| --- | --- |
| `just dev` | Everything, rebuilt on save. With a Studio checkout beside this one (`../studio`, or `LOCAL_STACK_STUDIO_PATH`), **Studio's stack** with this checkout as its Architect: Studio's node with Studio's extensions linked in, on a copy of a real setlist vault and its share link, the issuer, the web app and the site (`studio:docs/local-stack.md`). Arguments go to Studio's (`keyflow=local`, `desktop=on`, `watch=off`). `just dev plain`: this repo's own stack below (`devenv up mock-oauth auth node app web site`). |
| `just demo` | The quickest look, nothing watched: the issuer and a node with a share link (Studio's when it is beside this one, else the demo seed's, `/s/ksqx75iaaaaaaaae/`). Sign in as `ada@local.test`, password `development-password`. |
| `just website` | The public site alone (`apps/site`: landing, download, changelog, extensions, docs) with hot reload, on 7414. `just docs-serve` still serves `docs/content` alone. |
| `just phone` | Studio's running stack's node on this tailnet over HTTPS (`tailscale serve`, never funnel), for a phone. |

`devenv up` with no arguments is unchanged: this repo's own stack.

### A share page's assets without relinking

A debug build of the node reads `site.css`, `site.js` and `player.js`
from the folder `ARCHITECT_SITES_ASSETS_DIR` names, at every build of a
site's pages, instead of the copies compiled in
(`features/sites/src/assets.rs` `DEV_ASSETS_VAR`); a release build
ignores it. Studio's stack sets it to a local Architect's
`features/sites/assets` and rebuilds the share pages on a save, so an
edit to the player shows on reload in about three seconds, with no
build. A linked extension's renderer has no such copy on a node yet:
#626.

## What starts

Each process is `cargo xtask dev run <process>` (`xtask/src/dev/mod.rs`),
in this order, each waiting for the one before it to answer:

| Process | Port | Waits for | What it is |
| --- | --- | --- | --- |
| `mock-oauth` | 7410 | | The stand-in for GitHub, Google and TONE3000 (`apps/mock-oauth`): the auth server's social buttons work with nobody's credentials. |
| `auth` | 7411 | `mock-oauth` | The auth server (`apps/auth-server`): SQLite on disk under the stack's state, a secret of its own generated on its first start, filled as below. |
| `node` | 7412 | `auth` | `architect-desktop serve`, built without the window: signed in to the local auth server, hosting the fill's org and a vault of the demo's content, `ARCHITECT_IROH=local`, files in a local folder. |
| `app` | (a window) | `node` | The Architect app under `dx serve`: its issuer the local auth server, its orgs' node the local node (`ARCHITECT_ORG_HOST`), a folder of its own with its sessions in a file there (never the keychain), a scratch copy of the demo vault. |
| `web` | 7413 | `auth` | Off by default (`devenv up web`): the web front end (`scripts/web-bundle.sh`), its `config.json` pointed at the local issuer. |
| `site` | 7414 | | Off by default (`devenv up site`): `apps/site`'s dev server. |

The ports are `dev-stack.toml`'s, the one place to change them
(`devenv.nix` reads them from there too). They stay clear of what a fleet
machine runs: Teley on 8787, the collector on 4318, `just auth-dev` and
`just web` on 8080, `just mock-oauth` on 4040. `DEV_STACK_PORT_AUTH` (and
`_NODE`, `_MOCK_OAUTH`, `_WEB`, `_SITE`) moves one for a run.

Everything the stack keeps is under devenv's state, in
`.devenv/state/stack/` (`$DEVENV_STATE/stack`): the auth database and its
secret, the snapshot, the node's identity and data, the app's folder, and
`fill.json`, what was decided and why. The app's scratch vault is
`.devenv/state/demo-vault`, as before.

`just dev-status` says what is running, the fill and why, the snapshot's
age, the URLs to open and the development password.

## Two ways to fill it

The auth server decides when it starts, once, and says so in a banner at
the top of its log:

1. **A credentialed machine**: an administrator's session token is in
   `AUTH_ADMIN_TOKEN`, or in the file `AUTH_ADMIN_TOKEN_FILE` names. The
   stack mirrors production (`dev-stack.toml` `[mirror] from`): one
   authenticated GET of `/admin/snapshot`, bounded at 30 seconds. The
   server scrubs the snapshot before it leaves production
   (`features/auth/auth-db/src/snapshot.rs`): no password hashes,
   sessions, OAuth tokens, API keys, passkeys or two-factor secrets. What
   arrives is the organization graph: accounts, orgs, memberships, roles,
   teams, invitations. Addresses are redacted to
   `user-<id>@local.invalid` unless you ask for real ones
   (`just dev-refresh --real-emails`). The local auth server imports it
   (`AUTH_IMPORT_SNAPSHOT`, the same import `just auth-replay` uses), and
   every account gets the development password, `development-password`,
   so anyone on the machine can sign in as anyone, here only. The node
   hosts the org `[mirror] node_org` names (the first by slug with an
   owner when empty) and signs in as its owner.
2. **Everyone else**: the demo seed. The auth server's fixed cast
   (`apps/auth-server/src/dev.rs` `DEV_PEOPLE`, `DEV_ORGS`): Ada owns
   `acme` and is the administrator, Grace is an admin of `acme` and owns
   `indie`, Alan is a member of `indie`. The node signs in as Ada and
   hosts `acme`, with the demo vault (`apps/ui/demo-vault`) and the
   seed's additions (`scripts/dev-stack/seed-vault`): a setlist,
   `Setlists/Fall Show`, with a chart per part, and a share link per
   audience kind, the same codes on every machine:

   | Link | Audience | Opens |
   | --- | --- | --- |
   | `/s/ksqx75iaaaaaaaab/` | public | `Areas/Music` |
   | `/s/ksqx75iaaaaaaaac/` | anyone with the link | `Songs` |
   | `/s/ksqx75iaaaaaaaad/` | `acme`'s signed-in members | `Projects/Album` |
   | `/s/ksqx75iaaaaaaaae/` | anyone with the link | `Setlists/Fall Show` |
   | `/s/ksqx75iaaaaaaaaf/` | its `violin` audience | the violin parts only |
   | `/s/ksqx75iaaaaaaaag/` | its `cello` audience | the cello parts only |

   `apps/desktop/tests/dev_seed.rs` keeps the share notes equal to what
   the node writes.

If the mirror fails (no network, an expired token, production down), the
banner says why and the stack uses the last snapshot on the machine, or
the demo seed when there is none. `devenv up` never waits on production
for more than the mirror's 30 seconds.

A mirror is taken at every start: the local auth database is made again
from it, so changes you made to accounts locally last until the next
start. On the demo seed the database is kept between starts.

- `just dev-seed`: the demo seed from the next start on, even on a
  credentialed machine.
- `just dev-refresh`: mirror again now (and undo `dev-seed`); the next
  start loads it.
- `just dev-reset`: remove everything the stack keeps. Stop `devenv up`
  first; both `dev-reset` and `dev-seed` refuse while it runs.

## Making a machine credentialed

The token is an administrator's session at production's auth server. It
is never typed on a command line, never in the repo, and never in the
stack's state: the stack reads it from the environment or a file at the
moment it mirrors, hands it to curl on its stdin (never as an argument,
where the process table would show it), and strips `AUTH_*` from every
process it starts.

- **A fleet machine**: the token is a secret in the fleet's secrets
  (sops), placed on the machine by sops-nix the way every other secret
  is, readable by the developer's user only. A sops-nix secret lands at
  `/run/secrets/<name>`; name that file in the developer's environment,
  for example in `devenv.local.nix` (git-ignored):

  ```nix
  { env.AUTH_ADMIN_TOKEN_FILE = "/run/secrets/architect-dev-admin-token"; }
  ```

  Adding the secret to the fleet's secrets is a request to whoever keeps
  them, not a step in this repo.
- **Your own session**: sign in to production's auth server as an
  administrator, copy the session token (the `architect_session` cookie),
  and put it in a file only you can read; point `AUTH_ADMIN_TOKEN_FILE`
  at it. `AUTH_ADMIN_TOKEN` in your shell works too, for a one-off.

## Isolation, and what enforces it

Every guarantee is code, run before any process starts, and tested:

| Guarantee | Enforced by | Proved by |
| --- | --- | --- |
| Every URL, issuer, bind, peer and bucket endpoint a process is given is loopback or a private dev address; none names `*.fasttrackstudio.app`. A bind must be loopback. | `xtask/src/dev/guard.rs` `check_settings` over every process's environment; the only off-machine call is the mirror's GET, which is no process's setting | `xtask/src/dev/guard.rs` tests; `xtask/src/dev/mod.rs` `a_port_override_that_points_at_production_is_refused`; `xtask/tests/dev_stack.rs` (a stack pointed at a stand-in production is refused) |
| The dev shell carries no production `AUTH_SECRET`, database URL or bucket credentials. | `guard::check_inherited` refuses a shell that exports `AUTH_SECRET`, `AUTH_SECRET_FILE`, `AUTH_DATABASE_URL`, `AUTH_DATABASE_URL_FILE` or any `ARCHITECT_BLOB_S3_*`; every process is started with no inherited `ARCHITECT_*`, `AUTH_*`, `AWS_*` or `MOCK_OAUTH_*` and only what the stack sets; the node has no bucket, its files are a folder | `production_credentials_in_the_shell_are_refused`; `xtask/tests/dev_stack.rs` |
| The local auth server's secret is its own, generated once; a session minted here is worthless in production, and production's here. | `prepare_auth` writes 32 random bytes to `auth/secret` (0600) and gives the server `AUTH_SECRET_FILE` | `xtask/tests/dev_stack.rs` `a_credentialed_machine_mirrors_production_and_sessions_never_cross` |
| The node cannot find or be found by a production peer: no relay, no lookup, loopback only. | `ARCHITECT_IROH=local` (`apps/desktop/src/node/mod.rs` `bind_iroh`); its status reports `relays` and `iroh_addrs` | `apps/desktop/tests/node.rs` `a_local_node_has_no_relay_and_listens_on_loopback_only`; `xtask/tests/dev_stack.rs` reads them from the running node |
| The node is never an enrolled peer of production: a local edit cannot reach it. | It signs in only to the local auth server and enrols nowhere (`ARCHITECT_ENROL_AT` and `ARCHITECT_PEERS` are never set, and stripped if inherited) | `xtask/src/dev/mod.rs` `the_node_runs_local_iroh_and_signs_in_locally` |
| Mirrored data at rest is the developer's alone. | Folders `0700`, files `0600` (`write_private`, `private_dir`); `.devenv/` is git-ignored | `state_files_are_private`; `xtask/tests/dev_stack.rs` checks the modes of the running stack |
| The app's local sessions never meet real ones, and the stack never opens the platform keychain. | Its sessions are a file in its own folder, the stack's (`ARCHITECT_SECRET_STORE=file:…`, `ARCHITECT_CONFIG_DIR`), with `ARCHITECT_FORBID_KEYCHAIN=1` | `xtask/src/dev/mod.rs` `the_stacks_app_never_opens_the_platform_keychain` |

`just dev-guard` runs the guard alone. The production hostnames are a
constant (`guard::PRODUCTION_DOMAIN`); only the stack's own test can add
one (`DEV_STACK_TEST_PRODUCTION_HOSTS`, read only in a build with the
`stack-test` feature), to play production on a loopback port.

## What is and is not copied from production

| Copied | Not copied |
| --- | --- |
| Accounts (names, addresses, redacted by default), orgs, memberships and roles, teams, invitations, invite links | Passwords (every account gets the development password), sessions, OAuth tokens and linked accounts, API keys, passkeys, two-factor secrets |
| | Vaults, files, share links, devices and enrolments: the node holds the demo content |

### Vaults from production

Not yet. A credentialed machine's node holds the demo vault, under the
mirrored org. Pulling a production vault safely needs what production
does not have today:

1. a **read-only, one-shot export** of a vault on the production node: an
   endpoint (or action) an org administrator calls that answers the
   vault's notes and attachments as one archive, never as a sync session,
   and leaves out its share notes and device records;
2. an **import** on the dev side that writes that archive into the local
   node's vault under the local node's own identity;
3. a named allow-list of vaults in `dev-stack.toml`, empty by default,
   behind `just dev-refresh --vault <name>`.

What the stack must never do instead is join production's sync: an
enrolled or paired dev node would carry local edits back. Until the
export exists, `just dev-refresh --vault <name>` says so and stops.

## Sessions, and the keychain

No development run or test touches the platform keychain. A binary rebuilt
again and again is unsigned, a new code identity to macOS each time, so
reading a keychain entry an earlier build wrote makes the system ask the
person at the screen ("wants to use your confidential information stored
in …"), on every launch, and "Always Allow" cannot stick. So the stack's
app keeps its sessions in `secrets.json` in its own folder
(`ARCHITECT_SECRET_STORE=file:<path>`), any debug build on macOS or the
iOS simulator does the same in its app folder unless told otherwise, and
every `cargo test`, `cargo nextest` and `cargo run` in the repo runs with
`ARCHITECT_FORBID_KEYCHAIN=1` (`.cargo/config.toml`), under which
`architect_accounts::Keychain` panics rather than open it
(`features/accounts/src/keychain.rs` `secret_store`, `FileSecrets` in
`features/accounts/src/secrets.rs`). A release build keeps its sessions in
the keychain, under `ARCHITECT_KEYCHAIN_SERVICE` when set, and ignores
`ARCHITECT_SECRET_STORE` (except `memory`) and `ARCHITECT_FORBID_KEYCHAIN`:
whoever can set its environment cannot move its sessions into a file.

## Running the pieces alone

The one-off auth servers share the stack's code: `just auth-dev`,
`just auth-dev-persistent` and `just auth-replay` are
`cargo xtask dev auth` (one function writes the auth server's environment
for all of them), on port 8080 with the published development secret.
`cargo xtask dev env node` prints what the stack gives a process (no
secret is in it: secrets are files).

The stack's test, `xtask/tests/dev_stack.rs`, starts the same
`cargo xtask dev run` commands in a temporary folder on free ports, with
no devenv: `cargo xtask feature-tests --run --filter dev-stack`.
