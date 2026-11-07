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
| `web` | `ghcr.io/fasttrackstudios/architect-web:<X.Y.Z>`, `:sha-<short>`, `:latest` (the landing page, the docs and the web app, [the web front end](@/guide/web.md)), and `architect-web-<X.Y.Z>.tar.gz` |
| `linux` | `Architect-linux-x86_64.AppImage` (`scripts/desktop/linux.sh`) and `latest-linux.json`: the version, the AppImage's URL and its sha256, for the updater |
| `macos` | `Architect-macos-arm64.dmg`, Developer-ID signed and notarized (`scripts/desktop/macos.sh`), and `latest-macos.json`. Untested, and off until the repo variable `ARCHITECT_MACOS_RELEASE` is `true` |
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
