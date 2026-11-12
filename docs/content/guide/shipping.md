+++
title = "Shipping"
description = "Versions are tags: cutting one on a green main commit, the tag-order check, the release workflow, and consumers pinning one tag."
weight = 70
+++

## What you'll do

Release architect (or an app built the same way) and move its
consumers onto the release. A release is a git tag; there is no release
branch, no release PR and no version bump commit. This chapter covers
cutting a tag, the check that keeps tags ordered, what the release
workflow publishes, how a consumer pins a tag, and how to move every
architect crate a consumer uses to a new one at once.

## 1. Versions are tags

Consumers (Task, studio and the other fleet repos) depend on architect
as git dependencies pinned to a tag. From Task's root `Cargo.toml`:

<!-- doc-check: off (another repo's manifest; the tag is an example) -->
```toml
architect = { git = "https://github.com/FastTrackStudios/architect", tag = "v0.9.30", features = ["iroh"] }
architect-telemetry = { git = "https://github.com/FastTrackStudios/architect", tag = "v0.9.30" }
auth = { git = "https://github.com/FastTrackStudios/architect", tag = "v0.9.30" }
```
<!-- doc-check: on -->

So the tag is the version. `[workspace.package] version` in architect's
root `Cargo.toml` is not the release number and does not move. Tags are
`vX.Y.Z`, and several land a day; `gh release list -R FastTrackStudios/architect -L 3`
shows the latest.

Every architect crate a consumer uses must be on the same tag, and the
consumer's `Cargo.lock` must hold one source for each. Two tags in one
lockfile means the `auth` crates were compiled against a different
`architect` than the app uses; two vox revisions mean the server and
its clients fail the handshake at the first call. `cargo xtask
boundaries` in the consumer checks the single source.

Before 1.0, a tag may change public API. A PR that does says so in its
body, and updates `skills/architect/` in the same PR.

## 2. Only tag a green main commit

The PR checks are only fmt and the lockfile; the full suite (clippy,
tests, wasm, boundaries, docs) runs on `main` after the merge
(`.github/workflows/checks.yml`). A commit is releasable when its
`checks` run on `main` finished with the `check` job green. Tag that
commit, never a branch head or a commit whose run is pending or red:

```sh
gh run list -R FastTrackStudios/architect --workflow checks.yml --branch main -L 5
git fetch origin
git tag v0.9.32 <green-main-commit>
git push origin v0.9.32
```

Never move, delete or renumber a published tag: consumers already
resolved it. A bad release is fixed by the next tag.

## 3. Tags must order the history

A consumer's only question about a tag is "am I ahead of or behind
that other repo?", and that has an answer only if a higher number is
always a later commit. `cargo run -p xtask -- tags --verbose` (also
`just tags`) checks that every `vX.Y.Z` tag descends from every
lower-numbered one (`xtask/src/tags.rs` `run`). CI runs it on `main`
with the full history; a shallow clone gives false failures.

One historical violation is grandfathered (`v0.0.2`, which a consumer
still pins) in `GRANDFATHERED`, and that list must never grow. Tag from
`main` in order and the check passes by construction.

## 4. The release workflow

Pushing a `v*` tag runs `.github/workflows/release.yml` on the
self-hosted `thebattleship-release` runners. The notes are the change
files added since the previous tag (`scripts/release-notes.sh`, `just
release-notes`). Architect, the app, is released from the same tag
(#431):

| Job | What the release gets |
| --- | --- |
| `release` | the GitHub release and its notes |
| `node` | `ghcr.io/fasttrackstudios/architect:<X.Y.Z>`, `:sha-<short>`, `:latest` (the headless node, [deploy](@/guide/deploy.md)), and `architect-node-<tag>-x86_64-linux.tar.gz` |
| `web` | `ghcr.io/fasttrackstudios/architect-web:<X.Y.Z>`, `:sha-<short>`, `:latest` (the site, the docs and the web app, [the web front end](@/guide/web.md)), and `architect-web-<X.Y.Z>.tar.gz`, what the server serves. Waits for `release` only: the site's `/download` names the release's files by convention and checks them in the browser |
| `linux` | `Architect-linux-x86_64.AppImage` (`scripts/desktop/linux.sh`) and `latest-linux.json`: the version, the AppImage's URL and its sha256, for the updater |
| `macos` | `Architect-macos-arm64.dmg`, Developer-ID signed and notarized (`scripts/desktop/macos.sh`), and `latest-macos.json`. Built on airlock; on while the repo variable `ARCHITECT_MACOS_RELEASE` is `true` |
| `ios` | the Architect app for iPhone and iPad (`apps/mobile`) on TestFlight, marketing version `X.Y.Z` and a unix-time build number, noted in the release's notes (`.github/workflows/ios.yml`). Built on airlock; it never holds the release, and the repo variable `ARCHITECT_IOS_RELEASE` set to `false` turns it off |
| `nix-bump` | a PR (branch `chore/nix-desktop-<tag>`) pointing the flake's `architect-desktop` at the new AppImage, for the highest release only; it warns instead of failing |
| `sums` | `SHA256SUMS` over every asset, and `SHA256SUMS.minisig` signed with the org key when the `release-signing` environment has `FTS_RELEASE_MINISIGN_KEY` |

`:latest` goes only to the highest release. Packages pushed with
`GITHUB_TOKEN` start private on GHCR, so each image is made public once
in the org's package settings. A Windows build is a follow-up.

An app that ships binaries adds its own release workflow that builds,
checksums and signs them, and the binaries update themselves through
`architect::release` (feature `release`): it finds the newest
`<prefix>X.Y.Z` release, checks the minisign signature over
`SHA256SUMS` against the key the app compiles in, the asset's hash and
a `--version` smoke test, then swaps the binary atomically
([signed self-update](@/architecture/release.md)).

### The desktop app on Nix

The flake's `architect-desktop` (also `default`, x86_64-linux) is the
release's own AppImage run in an FHS env (`nix/desktop/package.nix`),
so installing it compiles nothing:

```sh
nix run github:FastTrackStudios/architect            # the newest pinned release
nix run github:FastTrackStudios/architect/v0.12.0    # a given tag
```

In a NixOS or home-manager configuration, add the flake as an input
and install its package:

```nix
inputs.architect.url = "github:FastTrackStudios/architect";
# then, in a module that has `inputs` and `pkgs`:
environment.systemPackages = [ inputs.architect.packages.${pkgs.stdenv.hostPlatform.system}.architect-desktop ];
```

It installs the app's desktop entry and icon, so it shows in app
launchers. The version and hash live in `nix/desktop/source.json`;
`nix/desktop/update.sh [<tag>]` rewrites them from a release's
`latest-linux.json`, and the release's `nix-bump` job opens that PR.

Updates reach a Nix user two ways, and the newer one runs. The store
is read-only, so the launcher (`nix/desktop/launch.sh`) sets
`ARCHITECT_UPDATE_DIR` (default
`${XDG_DATA_HOME:-$HOME/.local/share}/architect/updates`, kept when
already set) for the app's updater to write a downloaded
`Architect-linux-x86_64.AppImage` and its `version` file into. At each
start it runs whichever is newer, the store's image or the downloaded
one (`nix/desktop/choose.sh`), with `APPIMAGE` set to that file: a flake
bump (`nix flake update architect`) is never shadowed by an older
download, and an in-app update runs without waiting for the flake. The
store's image wins a tie, a release beats a nightly of the same
`X.Y.Z`, and a missing or unreadable `version` means the store's.
`nix/desktop/test-choose.sh` (the flake's
`checks.x86_64-linux.architect-desktop-choose`) covers the choice.

### The iOS app on TestFlight

`apps/mobile/ios/deploy-testflight.sh` builds the phone app for release
(`apps/mobile/ios/build-app.sh --device --release`: cargo with Xcode's
toolchain, the app icon and launch colour compiled from
`apps/mobile/ios/Assets.xcassets`), signs it with the Apple Distribution
identity and the `FTS Architect App Store` profile, validates it with App
Store Connect and uploads it. It takes Studio's environment
(`KEYCHAIN`/`KEYCHAIN_PW`, the `ASC_*` key from
`~/.appstoreconnect/config.env`, `PROFILE_NAME`, `DX_BUNDLE_ID`,
`MARKETING_VER`, `BUILD_NO`), so airlock serves both apps:

```sh
KEYCHAIN=fts-build.keychain KEYCHAIN_PW=fts-build MARKETING_VER=0.12.2 \
  apps/mobile/ios/deploy-testflight.sh --dry-run    # build, sign, validate; no upload
ruby apps/mobile/ios/testflight-status.rb           # the builds App Store Connect has, read-only
```

The release's `ios` job runs it for every tag; `gh workflow run ios.yml
-R FastTrackStudios/architect -f tag=vX.Y.Z` ships an existing tag (add
`-f dry_run=true` to stop before the upload). A build reaches testers
only once someone with App Store Connect access adds them to a TestFlight
group: the job adds no tester and submits nothing for review.
`apps/mobile/ios/app-icon.sh` redraws the icon from
`apps/desktop/icons/architect.svg`.

### The site

`architect.fasttrackstudio.app` is served by one VPS: its static file
server serves the release's `architect-web-<X.Y.Z>.tar.gz` and sends the
node's paths to the node (#534). The site is `apps/site`, a TanStack
Start app on bun with its own lockfile, biome and tsconfig, and it is the
root of that bundle (`scripts/web-bundle.sh`). Cargo never builds it.
Every page is prerendered, so none needs a server when it is requested:

| Page | From |
| --- | --- |
| `/` | the landing page; its words are in `apps/site/src/content/landing.ts`, its design tokens in `apps/site/src/styles/tokens.css`, its font in `apps/site/public/fonts` |
| `/download` | the release's tag: each file's link follows from it (`apps/site/src/lib/release-assets.ts`), and the page asks GitHub's API which files are up, with their sizes and sha256; also the Nix one-liner above and how to check a download against the signed `SHA256SUMS` |
| `/changelog` | the last releases' notes (the GitHub release bodies, which are `scripts/release-notes.sh`'s output) |
| `/extensions` | the store's index, `extensions/index.toml`. While it lists nothing, the page is built from `examples/registry/index.toml` and says it is an example |
| `/docs/` | `docs/content`, built by `architect site build --base /docs/` |
| `/app/` | the web app ([the web front end](@/guide/web.md)) |

`apps/site/scripts/data.ts` reads the releases and the index when the
site is built and writes `apps/site/src/data/*.json`, which is
committed. A local or pull-request build keeps the committed copy when
GitHub cannot be reached. A release's build is strict:
`release.yml`'s `web` job sets `ARCHITECT_RELEASE_TAG` and fails unless
that release exists and `/changelog` lists it with its notes. Both come
from the `release` job, the only one `web` waits for, so the bundle the
server deploys does not wait on the desktop builds.

`/download` carries no file data from the build. Every asset's name
follows from the tag (`Architect-linux-x86_64.AppImage`,
`Architect-macos-arm64.dmg`, `architect-node-<tag>-x86_64-linux.tar.gz`,
`architect-web-<X.Y.Z>.tar.gz`, `SHA256SUMS`, `SHA256SUMS.minisig`), so
the prerendered page links each one under
`https://github.com/FastTrackStudios/architect/releases/download/<tag>/`
and works with scripts off. On load it makes one call to
`https://api.github.com/repos/FastTrackStudios/architect/releases/tags/<tag>`
(unauthenticated, 60 an hour per address; kept in `sessionStorage` for
ten minutes) and reconciles: a file the release does not list yet (a
job still running, or the optional Mac build off or failed) is shown as
not available, and the listed ones get their size and sha256 (the API's
asset `digest`). If the call fails or is rate limited, the links stay as
they are. The page never fetches an asset itself. The landing page's
Download buttons do the same. The sha256 of each file is always in
`SHA256SUMS`, signed with the org key; the page shows how to check both.
`sums` still runs last, after `web`, so the sums cover the bundle.

The site's links to the web app (`/app/`) and to share links (`/s/…`)
are same-origin paths: the site and the node share the domain.
`scripts/web-routes.md`, copied into every bundle as `ROUTES.md`, says
exactly how a server must serve it: try the path, then `<path>.html`,
then `<path>/index.html`, else `404.html` with a 404; the node's paths
and `/auth/callback` ahead of the files; long caching only for the
content-hashed `assets/`. `apps/site/scripts/serve.ts` serves a folder
by exactly those rules, and `scripts/check-web-bundle.sh` checks a
bundle behind it: the files the server asserts, `/`, `/download`,
`/changelog`, `/extensions`, `/docs/`, `/app/`, the 404, and that `/` is
this site.

`.github/workflows/site.yml` checks a pull request that touches the
site, the web app's files, the docs or the bundle's scripts: biome,
tsc, the build, the route tree's drift, every internal link, then the
bundle assembled around this build with the latest release's docs and
wasm, through `scripts/check-web-bundle.sh`. It deploys nothing; the
release's `web` job runs the same check on the bundle it ships.
Locally, from `apps/site`, with bun (`nix shell nixpkgs#bun`):

<!-- doc-check: off (bun commands, run in apps/site) -->
```sh
bun install --frozen-lockfile
bun run dev                                     # http://localhost:3000
bun run build                                   # .output/public, every page prerendered
bun scripts/serve.ts .output/public 4173        # by the bundle's rules
```
<!-- doc-check: on -->

From the repository root, `just web` builds the whole bundle (the site
too) and serves it.

## 5. Moving a consumer to a new tag

`cargo xtask fleet-bump` rewrites every architect-owned git dependency
in a consumer's manifests to one tag (`xtask/src/fleet.rs` `run`):

```sh
just fleet-bump v0.9.32 ../task ../studio        # a directory means its Cargo.toml
cargo run -p xtask -- fleet-bump v0.9.32 ../task -n   # report only
```

It edits a line when its key is `architect`, `auth`, `crdt`,
`ui-snapshot` or one of their `-*` crates and the line names
`FastTrackStudios/architect`, replacing its `tag` / `rev` / `branch`
with `tag = "<TAG>"`. It does not touch `Cargo.lock`, does not check
that the tag exists, and skips a dependency split over several lines.
So, in the consumer:

1. Run the bump.
2. Refresh the lock (`cargo update -p architect`, or `cargo metadata`
   for the full set) and confirm one architect source in it.
3. Run its `boundaries` (it reads the lock with `--locked`), its wasm
   check and its tests.
4. Keep the server and every client on the same tag in the same
   change.

A consumer builds architect's xtask from the tag it pins:
`cargo install --locked --git https://github.com/FastTrackStudios/architect --tag <TAG> xtask`.

When the consumer needs something architect does not have yet, the
change goes into architect first, then a tag, then a bump. Don't copy
framework code into an app, and keep a `[patch]` onto an architect
branch to a labelled experiment.

## 6. The toolchain moves together

`rust-toolchain.toml` in architect and in each consumer pin the same
rust version (1.99.1 today), and devenv-fleet's pins give every repo's
shell and CI the same rustc.
Bump them together: a consumer on an older toolchain than the tag it
pins may not build it. The rustflags in `.cargo/config.toml` and the
dependencies' dev profile in the root `Cargo.toml` are shared the same
way: the build cache reuses a crate across repos only when the
compiler, every flag and the profile match.

## Where to read more

- [Signed self-update](@/architecture/release.md): `architect::release`
  and the org release key.
- [Monorepo layout](@/architecture/layout.md): the boundary check the
  consumers run.
- [Idioms §5](@/architecture/idioms.md): one vox revision across server
  and clients.
- [Feature flags](@/architecture/features.md): what a consumer turns on
  when it pins.
