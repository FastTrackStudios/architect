+++
title = "App updates"
description = "How the Architect app updates itself: a check at start and daily, Install on request, verified against the org-signed sums, written where the Nix launcher or the AppImage expects it."
weight = 66
+++

The Architect app keeps itself on the newest signed release through
Updates, a built-in extension (`features/updates`, id `updates`, #523).
It is linked into the Architect distribution
(`apps/distribution/src/lib.rs` `architect`); the checking, downloading
and verifying is `architect::release` (`release.md`).

## What a person sees

- One status item, only when there is something to say: **Update X.Y.Z
  available** (a press installs it), **Downloading X.Y.Z… 42%**, then
  **Restart to finish**. A check that finds nothing, or fails because the
  network is down, shows nothing and opens no dialog.
- Settings → Updates: the running version, the channel, the last check
  and **Check now**; Install, Restart or the release page when they
  apply; the channel and **Check automatically**.
- Help → Check for updates.

Nothing downloads until the person chooses Install.

## The release version

The binary knows which release it is only when the release build says
so: `cargo xtask dist desktop --version X.Y.Z` (what `release.yml`'s
`linux` and `macos` jobs run) sets the build-time env
`ARCHITECT_RELEASE_VERSION` for its cargo build, and
`features/updates/src/lib.rs` `RELEASE_VERSION` reads it with
`option_env!`. It is read in that one crate, so only that crate's build
cache key moves with each release.

The stamp is `X.Y.Z`, or a nightly's `X.Y.Z-nightly.YYYYMMDD`, ranked
as `nix/desktop/choose.sh` ranks them (a release beats a nightly of the
same `X.Y.Z`; `model::newer`). `cargo xtask dist --version` refuses any
other form (`0.13.0-rc.1`), which would build an app that never updates.

A build without it (`cargo run`, CI, a dev build) is a **development
build**: it never checks, offers or installs anything, and says so. The
workspace version (`CARGO_PKG_VERSION`, `0.1.0`) is never compared.

To see what a build says, without opening a window:

```sh
architect-desktop distribution describe | grep release_version   # "0.12.0", or null
architect-desktop actions run UPDATES_STATE                        # in a running app: current, update
```

## Actions

| Id | Effect | What it does |
|---|---|---|
| `UPDATES_CHECK` | mutating | Asks for the newest release now (the network; the state changes); answers the state. |
| `UPDATES_INSTALL` | destructive | Starts the download, verify and install of the available update, once; answers at once. |
| `UPDATES_STATE` | read-only | `current` (null in a development build), `development_build`, `channel`, `last_check`, `update`. |
| `UPDATES_RESTART` | destructive | Quits; a replaced `AppImage` starts again on the new version. |
| `UPDATES_PAGE` | mutating | Opens the release's page in the system's browser. |
| `UPDATES_SCHEDULED` | mutating | Its hooks' target (at start, every 24 h): a check when automatic checks are on. |

Install and Restart are destructive, so a caller from outside (a model
over MCP, a script run as someone) waits for a person's yes
(`crates/architect/src/action_list/confirm.rs`). The app's own status
item, menu and settings buttons are a person pressing them: never asked
twice.

Each change of state is one compare-and-set under one lock
(`features/updates/src/model.rs` `Machine`): a check never starts or
lands while an install is under way or done, and only an available
update starts an install, once.

`UPDATES_PAGE` opens only a page under
`https://github.com/FastTrackStudios/architect/releases/` (the URL comes
from the API's unsigned JSON; `model::release_page`), else that releases
page, with `xdg-open` (`open` on macOS, the URL handler on Windows).

The `update` state is one of `idle`, `development_build`, `checking`,
`up_to_date`, `available` (`version`, `notes_url`, `signed`, `plan`),
`downloading` (`received`, `total`), `ready` and `failed` (`reason`)
(`features/updates/src/model.rs` `State`).

Settings: `updates.channel` (`stable`; `nightly` is present and refused
as not available yet until nightly releases exist) and `updates.auto`
(default on).

## Where an update goes

`features/updates/src/model.rs` `plan` decides from the env and the
platform, in this order:

1. A development build: report only.
2. Not Linux x86-64 (macOS, Windows): report the update and its download
   page. Installing there is a follow-up.
3. `ARCHITECT_UPDATE_DIR` set (the Nix package's launcher,
   `nix/desktop/launch.sh`): the verified image is written to
   `$ARCHITECT_UPDATE_DIR/Architect-linux-x86_64.AppImage` (mode 0755),
   then its version (`X.Y.Z` and a newline) to
   `$ARCHITECT_UPDATE_DIR/version`, each to a temp name in that folder
   and renamed. An old `version` is removed before the new image is
   renamed in, and the folder is synced after each step, so `version`
   never names an image it was not written for; if writing the new one
   fails, the launcher (`nix/desktop/choose.sh`: no `version`, the
   store's) runs the store's image. The folder is made 0700 when missing.
   `$APPIMAGE` is never touched.
4. `$APPIMAGE` is the image this process runs from (`$APPDIR` set, the
   running binary under it, the file named `*.AppImage`) and writable by
   this user (a plain download): a temp file beside it, then a rename over
   it. An `$APPIMAGE` inherited from another app's image (a terminal
   opened inside it) is never replaced: report only.
5. Otherwise (a distro package, a bare binary): report only.

The restart: a replaced `AppImage` is started again once this process has
exited, by a watcher that gives up after 60 s and is stopped when the
quit fails (`features/updates/src/restart.rs` `start_after_exit`); under
the Nix launcher the app quits and the person opens it again (the
launcher then picks the new image).

Staging files are created 0600 and made 0755 only after verification,
just before the rename; one left by a run that died mid-download
(`.Architect-linux-x86_64.AppImage.update-*`) is swept before the next
download. The image is capped at 1 GiB (a larger `Content-Length` is
refused before a byte is read, a stream is cut off at the cap), the sums
and signature at 1 MiB; a download gives up after 60 s without a byte,
with no overall limit, so a slow link still finishes.

## What is checked

The release must be the newest non-draft, non-prerelease `vX.Y.Z` that
carries the platform's asset and `SHA256SUMS`, and newer than the running
version. Installing then needs `SHA256SUMS.minisig` by the org key
(`architect::release` `ORG_KEY`) whose trusted comment is
`architect vX.Y.Z SHA256SUMS`, and the image's sha256 to match the signed
sums as it streams in. An unsigned release is offered with a link to its
page and refused on Install ("vX.Y.Z is not signed … so it is not
installed"); a bad signature or a checksum mismatch is refused, and
nothing is left in the folder. No `--version` smoke test runs: an
`AppImage` cannot be asked its version headless.

## Feature `release`

The network half (reqwest, a tokio runtime of its own) is the crate's
feature `release`, which the app crate turns on
(`apps/distribution/Cargo.toml`), as it does `architect-extensions/net`:
the UI crate's own build stays out of the HTTP stack (`xtask boundaries`).
Without it the actions answer that the build cannot update itself.

## Tests

`features/updates/tests/install.rs` runs the engine
(`features/updates/src/engine.rs` `Engine`) against a local stand-in for
the GitHub API with a test signing key: an update is offered, installs
into the update dir (image, then `version`; `$APPIMAGE` untouched) or over
`$APPIMAGE`, an unsigned release, a bad signature and a checksum mismatch
are refused with nothing written, a prerelease is not offered on Stable,
Nightly is refused, no `$APPIMAGE` reports only, a development build never
asks. `features/updates/tests/model.rs` covers the plan, the stamp, the
status line, and the linked extension in a development build.

## Not yet

- Installing on macOS (a `.app` bundle) and Windows.
- The nightly channel and its key (#523 (d)).
- An end-to-end run against a real signed release: v0.12.0's sums went
  out unsigned (the release-signing secret is not set yet), so the app
  reports it and installs nothing.
