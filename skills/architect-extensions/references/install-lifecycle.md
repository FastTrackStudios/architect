# Installing, updating and shipping extensions

How an extension gets onto a device and stays current: development copies, registries, signed archives, install on open, a person's devices, core extensions and the node.

Docs: `docs/content/extensions/authoring.md` (the walkthrough), `docs/content/architecture/extensions.md` ("Hot reload", "Packaging", "Distribution", "Finding and installing", "What a project needs", "A person's installs on all their devices", "A share link's extensions"), `docs/content/guide/core-extensions.md`, `docs/content/guide/deploy.md` ("Extensions' node actions", "Extensions through Nix"), `docs/content/guide/showcase.md` (rows L1–L4).

## Which copy runs

An id can have three copies at once (`features/extensions/src/catalog.rs`):

| Copy | Where it comes from | Wins when |
|---|---|---|
| built-in (linked) | compiled into the distribution (`Extensions::with_linked`) | nothing else is there |
| installed | a registry, a file or a peer, one folder per id under the app's `extensions/` folder | it is newer than the built-in; among installed ones, the newest |
| development | a folder you point the app at (`EXT_DEV`, `EXT_CLONE`) | always, until its session ends |

What lost is kept, so the extensions page can say "built in, overridden by …" with **Use the built-in**. A copy that cannot run on this platform still counts as the copy of its id: an installed or development copy for another OS shadows a built-in that could run. Remove it, or end the development copy, to get the built-in back (`docs/content/architecture/extensions.md`, "Which operating systems: `platforms`").

Each install's folder holds `installed.toml` (`features/extensions/src/install.rs` `Install`): id, version, kind, source, an optional `link` (a folder registry's package is linked, not copied, so an edit there is what loads), `pin`, and `signer_key` / `signer`. The signed archive it came as is kept beside it.

Where installs are kept is a `Disk` (`features/extensions/src/disk.rs`): the app's folder on a desk and a phone (`OsDisk`), the browser's storage in the web app (`MemDisk`, each change handed to a callback that keeps it). `Installs::with_disk` and `Grants::open_on` take one; a package installed on a `MemDisk` reads through it (`features/extensions/src/package.rs` `Files::Disk`, `Package::texts`). A source with no folder to write into hands its package over as files (`Source::fetch_files`, `Fetched::Files`; the built-in registry `BundledSource` always does). Test: `features/extensions/tests/browser_installs.rs`.

What a platform has no runner for is "Can't run here" with why before anything installs (`features/extensions/src/host.rs` `why_no_runner`, `kind_cannot_run`): in a browser, processes and WASI components (and Luau only when the bundle carries no luau-web).

## Development copies and hot reload

| Way in | What it does | Code |
|---|---|---|
| `EXT_DEV {folder}` | loads a package folder unsigned as a development copy; reloaded on save; loaded again on the next start | `features/extensions/src/registry_actions.rs`, `Extensions::dev` |
| `EXT_DEV_END {folder}` | ends it; the folder stays | `Extensions::end_dev` |
| `architect-desktop extensions dev <folder>` / `undev <folder>` | the same two actions, over the running app's control socket | `apps/desktop/src/app.rs` |
| `architect-desktop extensions check <folder>` | loads it once on its own and renders each view once (`EXT_CHECK`) | `features/extensions/src/check.rs` `Checker` |
| `EXT_CLONE {id, as?}` | copies a built-in or installed one into the app's `dev/` folder under a new id (manifest, action ids and setting keys renamed) and runs it as a development copy | `features/extensions/src/clone.rs` `Extensions::clone_to_dev` |
| `EXT_CLONE {id}` on a built-in with `[source]` | writes its crate to the dev folder and builds its process binary; it overrides the built-in under the same id | `features/extensions/src/source.rs` |

Rules:

- **Development installs must be on.** `EXT_DEV` is refused unless Settings → Extensions has them on (`EXT_DEV_INSTALLS {on}`; turning it on from outside needs a person's yes) or the build is a debug build (`Registries::with_dev_forced`).
- **Change detection is polling, not file events.** `features/extensions/src/watch.rs` fingerprints every file's path and contents (`.git` left out). The app polls `Extensions::reload_changed` on a timer (`apps/ui/src/extensions.rs`, every 500 ms); `EXT_RELOAD` forces one pass.
- **A reload is a diff, not a restart** (`docs/content/architecture/extensions.md`, "Hot reload"): the old copy's state is saved, its contributions withdrawn as a diff, the new one gets the state back, and a view that is still there remounts in the same panes. A failed build or a panic keeps the last good copy and says why.
- **From-source clones** use the app's `cargo` (`$CARGO`, else `PATH`), so run the app from the repo's dev shell. A failed build keeps the last good program; the error is in `APP_STATUS`, `EXT_LOGS` (source `build`) and on the page. `apps/sandbox/board/tests/dev_loop.rs` runs the whole loop (ignored by default: it builds a crate).

How fast a save shows, by kind: data and themes instantly; Luau and TypeScript on the next run; WASM components rebuilt (seconds, state kept); processes rebuilt and restarted with state handed over. Full table: "Hot reload" in the architecture page.

### An instance of your own: `architect-desktop dev`

An agent (or a worktree) gets its own Architect, apart from the person's window and from every other one: its own settings folder, vault, control socket, file secret store (never the keychain), local-only iroh, and no global capture shortcut (`apps/desktop/src/dev.rs`). As many run at once as are asked for.

```sh
architect-desktop dev --extension ./my-ext --only navigator --open Showcase.md
architect-desktop dev list                 # name, pid, socket, log, vault of each
architect-desktop dev path                 # this folder's instance, with its `drive` command
architect-desktop dev stop [<name> | --all]
```

- **Its name** is the working folder's (its last part and a hash of the whole path), so each checkout has one; `--name` picks another. It lives in `~/Library/Caches/architect-dev/<app>/<name>/` (`$ARCHITECT_DEV_ROOT` moves it); `--fresh` starts it over.
- **What it opens:** the demo vault, seeded fresh (`--seed <dir>` another, `--vault <dir>` an existing one); `--open <path>` a note or file beside the first tab; `--layout <name>` a saved layout; `--run ACTION[=JSON]` actions after, in order.
- **What runs:** `--extension <dir>` loads a folder as a development copy (reloaded on save, as above); `--only <id,id>` turns every other extension off (the development copies stay on); `--bare` turns them all off. With neither, everything but `updates` runs. The vault's install offer is not shown: the instance has what it was given.
- **Driving it:** the command prints JSON with `socket`; `architect-desktop actions --app <socket> …` drives that instance only (`APP_SHOT`, `APP_STATUS`, `EXT_LOGS`). Run again with the same name, it applies the new flags to the running one.
- **From the environment:** `ARCHITECT_DEV=1` (or a name) makes a plain launch an isolated instance, so a devenv or a script can set it; `ARCHITECT_DEV_ONLY`, `ARCHITECT_DEV_EXTENSIONS` (a path list), `ARCHITECT_DEV_OPEN`, `ARCHITECT_DEV_LAYOUT`, `ARCHITECT_DEV_VAULT`, `ARCHITECT_DEV_SEED`, `ARCHITECT_DEV_FRESH` and `ARCHITECT_DEV_RUN` (JSON `[["ACTION", {args}], …]`) are the flags' equivalents. The window's title says `(dev: <name>)`.

## Registries

`features/extensions/src/registries.rs` `Registries` keeps the registries an app searches and installs from, kept in `<app dir>/extension-registries.json`:

- **The store's index**: `extensions/index.toml` in this repo, given in code (`apps/distribution/src/lib.rs` `STORE_REGISTRY`; `apps/ui/src/extensions.rs` `app_registries` via `Registries::with_builtin`). Always there, never written or removed. A git registry checked out sparsely (`GitRegistry::in_dir`). A pull request that changes it is checked by `.github/workflows/extension-index.yml`.
- **Registries a person adds**: Settings → Extensions, or `EXT_REGISTRY_ADD {name, git [+ dir] | folder, keys?: [{name, key}]}`. Adding keys from outside the app needs a person's yes (`confirmation_required`).

Source types (`features/extensions/src/install.rs`): `GitRegistry` (a git repo with an `index.toml`, pulled with the `git` program), `FolderRegistry` (a folder with an `index.toml`, for a registry being written or on a shared drive), `ReaPackSource` (a ReaPack `index.xml`).

| Action | What it does |
|---|---|
| `EXT_REGISTRIES` | every registry, its keys, whether dev installs are on |
| `EXT_REGISTRY_ADD` / `EXT_REGISTRY_REMOVE` | add or replace by name / remove one the person added |
| `EXT_REGISTRY_REFRESH {name?}` | pull the indexes (the app does once at start, off the window's thread) |
| `EXT_SEARCH {query}` | the newest of each match across every registry |
| `EXT_PREVIEW {id, version?}` | what it would be and do, fetched into staging, nothing installed or run |

Examples to copy:

- `examples/registry/`: the skeleton of a real registry (`index.toml`, `index.schema.json`, `README.md`, a sample archive signed by a throwaway key in `keys/`).
- `examples/extension/registry`: a folder registry with one Luau package (`example.hello`) for the host example in `examples/extension`.
- `examples/showcase-registry`: signed archives, a fixture vault and a macOS-only extension, used by `features/extensions/tests/showcase_registry.rs` (showcase rows L1–L4).

An index entry per version carries `archive` (a release asset URL or a path in the repo) and its `sha256`, `platforms`, `changelog`; older entries may use `path` or `git` + `rev`. `cargo xtask ext-index <dir>` checks an index: schema, each archive's hash and signature, and that the archive is what its entry says (`features/extensions/archive/src/registry.rs` `check`).

## The `.archext` archive and trust

An `.archext` is a gzipped tar: `archext.json` (the content index: id, version, kind, target triples, every file's size and SHA-256), `archext.json.minisig` (minisign over the index, trusted comment `archext <id> <version>`), the package's files, and `bin/<target-triple>/<program>` per platform for a process extension. Packing is deterministic. Format: `features/extensions/archive/src/lib.rs` (`pack`, `open`, `peek`, `Verified::extract`, `ContentIndex`, `ArchiveError`); signing is that crate's `sign` feature (`features/extensions/archive/src/sign.rs` `sign_archive`), which apps never turn on.

Who is trusted:

- **The org release key** by default: `ORG_KEY` in `features/extensions/archive/src/lib.rs`, held by `Installs::new` (`Trust::org`); `Installs::with_trust` replaces it.
- **A registry's publishers** (`Publisher`: a name and a minisign `.pub` line), for that registry's archives only (`Source::trust`, `FolderRegistry::trusting`, `GitRegistry::trusting`).
- **A file install** (`EXT_INSTALL_FILE {path}`) is checked against the org key and every registered publisher's (`Installs::read_archive_trusting`).
- **The signer is pinned per id.** Each install records `signer_key`; an update signed by any other key is refused (`InstallError::SignerChanged`), from a registry or a vault.

Refusals happen before anything is written, each a typed error: `InstallError::Mismatch` (the index's sha256), then `ArchiveError::Unsigned`, `UntrustedKey` (names the key and the keys trusted), `IndexTampered`, `FileTampered`, `Unlisted`, `Missing`, `NotAnArchive`, `Unsafe`, `NoBuild`. Tests: `features/extensions/archive/tests/archive.rs`, `features/extensions/tests/distribution.rs`, `features/extensions/tests/registries.rs`.

Tooling (`xtask/src/ext.rs`):

| Command | Use |
|---|---|
| `cargo xtask ext-pack <dir> [--out F]` | the unsigned archive of a package folder |
| `cargo xtask ext-sign <archive> --key <file> [--expect <pub>]` | sign it (`EXT_SIGNING_KEY` text instead of `--key`); `--expect` refuses any other key |
| `cargo xtask ext-verify <archive> [--key <pub>]... [--no-org]` | open it as an app would |
| `cargo xtask ext-keygen <dir>` | a throwaway key pair, never a release key |
| `cargo xtask ext-index [dir]` | check a registry's index |

A third party (Studio) signs with its own key and lists the public half in its registry entry. A first-party release uses the reusable `.github/workflows/extension-release.yml` (plan, build per platform, pack, sign in the caller's `release-signing` environment with `--expect` the org key, index PR). See "Releasing one" in the architecture page.

## Install, enable, disable, pin, update, remove

Every operation is an Action List action (⌘K, a key, the CLI, MCP). Tables of all of them: `features/extensions/src/actions.rs`, `features/extensions/src/actions_manage.rs`, `features/extensions/src/contrib_actions.rs`, `features/extensions/src/registry_actions.rs`.

| Action | Arguments | Effect |
|---|---|---|
| `EXT_LIST` | | every extension, which copy runs, `platforms`, `not_here` |
| `EXT_INFO` | `{id}` | which copy runs, versions, pin, grants, what it adds |
| `EXT_INSTALL` | `{id, version?, grant?, explicit?}` | install and load at once; `needs_consent` with a preview when it asks past its grant |
| `EXT_INSTALL_FILE` | `{path}` | a signed `.archext` from a file or a peer |
| `EXT_GRANT` / `EXT_REVOKE` / `EXT_GRANTS` | `{id, scope?}` | grant (device or org), revoke at once, list |
| `EXT_ENABLE` / `EXT_DISABLE` | `{id}` | load or unload it and everything it adds |
| `EXT_PIN` | `{id, version?}` or `unpin` | keep it within a semver range (its current version when none) |
| `EXT_UPDATE` | `{id?}`, `{check: true}` | newest the pin allows; `check` only reports (version, changelog, `past_pin`, `asks_more`) |
| `EXT_UNINSTALL` | `{id}` | remove it; a built-in of that id comes back |
| `EXT_LOGS` | `{id, since?, limit?}` | recent log lines and errors |

Rules that bite:

- **Consent.** An install that asks for more than was granted answers `needs_consent`; install again with `grant` (`device`, `org`). An agent's `grant` or `EXT_GRANT` answers `confirmation_required` and a ticket: a person says yes (`features/extensions/src/manage.rs`).
- **Updates stop at growth.** An update whose capabilities grow waits in `needs_consent` (`Extensions::update`).
- **No silent downgrade.** Going older or past a pin by id is `InstallError::Downgrade` unless a person names the version (`EXT_INSTALL {explicit: true}`).
- **Not here.** An extension whose `platforms` exclude this OS is listed and kept, never started (`LoadError::NotHere`); the store refuses to install it (`InstallError::NotHere`). Tests: `features/extensions/tests/platforms.rs`.
- **The manager page** (`features/extensions/src/page.rs` `ManagerPage`) does all of the above with the same components; agents also get `MANAGER_LIST` / `MANAGER_RUN` (`features/extensions/src/manager/mod.rs` `Manager`).

## Install on open: a vault, a project, a share link

A vault names what it needs in `.architect/extensions.toml`; a project in its own folder's `.architect/extensions.toml`. Both are `Requirements` (`features/extensions/src/doc.rs`):

```toml
[[extensions]]
id = "fts.metronome"
name = "Metronome"
version = "^0.1"     # "*" for any
required = true      # false: offered, not asked
```

- `Extensions::open_vault` plans it (`features/extensions/src/vault_needs.rs` `VaultNeeds`); `EXT_NEEDS {folder}` gives an agent the same plan (ready, to install, to offer, missing with why, refused). Neither changes anything.
- A registry can be compiled into the app: `features/extensions/src/bundled.rs` `BundledSource` (packages whose files the build carries, listed and installed like a folder registry's path packages, through the same consent; the app's build vouches for them). The app carries the showcase's Luau, TypeScript and data packages this way (`apps/ui/src/showcase.rs` `source`), so the showcase vault's **Install all** works on the desk and on a phone with no network.
- The app draws it over the vault (`apps/ui/src/extensions.rs` `VaultNeedsBanner`, with `MissingExtensions`): required, optional, what cannot be had and why, and **Install all**. `Extensions::previews_for` builds one consent prompt; nothing installs before that one yes (`Extensions::accept_vault`), then everything loads without a restart. "Not now" hides it for the run.
- What cannot be installed is said, not forced: `Missing::WantsOlder`, `Missing::OutsidePin`, `Missing::CannotRun`.
- Tests: `features/extensions/tests/vault_needs.rs`; showcase row L2.

### Carrying extensions in the vault

Small extensions travel as signed archives in `.architect/extensions/*.archext` (`vault_needs::Carried`). Stricter than a registry:

- a source for that vault's plan only, never one of the app's registries; a registry's copy wins a tie;
- signed by the org key or by the key the installed copy of that id is already signed by: **a vault cannot bring a new publisher**;
- at most 16 MB an archive (`MAX_ARCHIVE`), 64 KB for the needs file (`MAX_NEEDS`); a plan reads only the signed index and manifest (`architect_extension_archive::peek`);
- no link is followed at any level, `.architect` included.

`.architect/` syncs with the vault's files (the sync client keeps that one top-level dot-folder: `features/vault/vault-sync-client/src/lib.rs` `SYNCED_DOT`).

### A share link's extensions

A share link names its own, not the vault's: `_architect/shares/<code>.extensions.toml` beside the share's note, packages in `_architect/shares/<code>.extensions/` (`features/extensions/src/share_needs.rs`).

- The owner sets it: `EXT_SHARE_NEEDS_SET {folder, code, extensions: [{id, required?, hosts?}]}` (`features/extensions/src/share_pick.rs` `Extensions::set_share_needs`); `EXT_SHARE_NEEDS` reads it back. A development copy or unsigned install is refused; so is a host its manifest does not ask for.
- The guest host uses the wasm-clean model: `LinkNeeds::from_toml`, `plan_link`, `verify_package`, `guest_capabilities` (read-only share documents, no writes, `fetch` only to approved hosts, no `[[secrets]]`).
- Only the share's owner may write those files: the node's write guard (`features/vault/vault-live/src/write_guard.rs` `ExtensionPaths`).
- **Not wired yet:** no host runs a guest's extensions (showcase row G1; #495, #755). Tests of the model: `features/extensions/tests/share_needs.rs`.

## A person's installs on all their devices

Owner decision (#683): what a person installs and grants on one device installs on their others with the same grants and no new prompt; removals and revokes travel too.

- The record is `.architect/people/<account id>/extensions.toml` plus `extensions.toml.minisig`, in the vault's synced `.architect/` (`features/extensions/src/person_sync.rs` `PersonInstalls`).
- **Signed by the device that wrote it.** Each device has a key (`DeviceKeys`, `<app dir>/device-key`); a device applies only records signed by itself or a device the person trusted once (`DeviceKeys::trust`, "It's mine: trust it"). Anything else is refused whole (`RecordError`). The node refuses anyone but that person a write under their folder (`ExtensionPaths`).
- Each entry is the last word on its id (`at_ms`): version, pin, signer, archive (in `.architect/extensions/`, SHA-256 pinned), grants, or `removed`. `Extensions::sync_person` publishes each change; `Extensions::apply_installs` applies newer entries. Same archive rules as a vault; grants are written only after the install succeeds.
- `EXT_INSTALLS_PUBLISH` / `EXT_INSTALLS_APPLY` (feature `person-sync`) act for the signed-in person only.
- Grants never transfer between people: other members and guests get the needs list and one prompt.
- Tests: `features/extensions/tests/person_sync.rs`.

## Core extensions (the standard release)

The owner picks which extensions the standard release includes; each lives in its own repo and is listed by reference in `extensions/core-extensions.toml` (schema `extensions/core-extensions.schema.json`). Never copy an extension's code into this repo.

Each `[[core]]` entry pins `id`, `publisher`, `source`, `version`, `tag`, `rev`, a `[core.signature]` (the publisher's minisign over the release's `SHA256SUMS`), and a form per platform: `[core.linked]` (compiled in from its crate at `rev`) or `[core.packaged]` (a signed release file shipped beside the binary, today a share-page island folder).

To add, bump or remove one, only when the owner asks:

1. Edit `extensions/core-extensions.toml` (`version`, `tag`, `rev`, signature URLs and comment, any `archive` + `sha256`).
2. `cargo xtask core verify` (network: tags name their revs, sums signed, archives listed).
3. `cargo xtask core sync` (regenerates `dist/standard/Cargo.toml`, `dist/standard/src/list.rs` and the lock).
4. Commit the list and `dist/standard` together. A bump that changes declared capabilities or the publisher key says so at the top of the PR.

`cargo xtask core check` is the offline CI check; `cargo xtask core graph` fails on a second Architect or a Task crate; `cargo xtask core fetch --platform node --into <dir>` fetches packaged folders. `dist/standard` is outside the workspace and built by `.github/workflows/standard.yml` (not required). Bundled `.archext` packages in the standard build are the next step, not built yet. Details: `docs/content/guide/core-extensions.md`; code: `xtask/src/core_list.rs`.

## Production: the node through Nix

Owner decision: the production node gets its extensions through Nix, pinned and verified at build time; the server downloads nothing at run time.

- `lib.architectExtension` (`nix/node/extension.nix`) fetches the archive (`url`, `registry`, or `src`), checks the pinned `hash`, then runs `archext install` (`features/extensions/archive/src/bin/archext.rs`): the same `open` every app runs, against `publishers` (and the org key with `trustOrg = true`), and the pinned id and version. Unsigned, unpinned or changed archives fail the build.
- `services.architect-node` (`nix/node/module.nix`) points `ARCHITECT_EXTENSIONS` at one read-only store path linking each extension by id; a change restarts the node. `extensionSecrets` names the folder of node-action secrets (`<dir>/<ext id>/<secret id>`, as sops-nix writes them).
- Checks: `nix/tests/extensions.nix` (`nix flake check`): the showcase's Bank link builds; a wrong hash, an unknown key and a tampered archive each fail.
- What runs there: only an extension's **node actions** (`runs = "node"`, Luau, holding a declared `[[secrets]]`), served on the org lane as `NodeActions` (`features/extensions/src/node.rs` `NodeHost`; `apps/desktop/src/node/mod.rs` `node_actions`). The deployment that names the extension is the node's consent. Tests: `features/extensions/tests/node.rs`.
- Not built: the desktop reading a declared extension list from Nix (`docs/content/guide/deploy.md`, "Extensions through Nix").

## Per platform today

| | Install from a registry | Install on open | Dev copy |
|---|---|---|---|
| Desktop | yes (L1) | vault and project folders yes; share links no (G1) | yes |
| Web | the registry the app carries (the showcase's), TypeScript and data; kept in the browser's storage (`apps/web/src/app/installs.rs`) | the built-in demo and showcase vaults (`VaultNeeds`); signed-in vaults no | no |
| iOS | Luau, pages, data, WASM (Pulley); not shown | same banner; not shown | through the control socket |

Current status per capability: `docs/content/guide/showcase.md`.
