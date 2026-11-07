+++
title = "Drive"
description = "architect-files' drive: every project and asset library a device knows, mounted as a real folder (FUSE), pinned roots at local speed, the rest fetched on open and kept for offline use."
weight = 27
+++

## What it is and when to use it

The drive mounts a device's roots as a real folder, the way Dropbox or
Google Drive for desktop do, so REAPER, a file manager, a file picker or
`rsync` reads and writes them like any other folder:

```text
~/Architect/<org>/Projects/<project>/…
~/Architect/<org>/Assets/<kind>/…
```

- **A pinned root** is its working copy, passed straight through. Reads
  and writes go to the local files, and the engine syncs them the way it
  syncs any edit (a version once the file has settled).
- **An unpinned root** is its manifest: real names, sizes and times, which
  browse offline.
  - **Opening a file** fetches it from a peer in the background and serves
    reads as the bytes arrive, so the first bytes come before the whole
    file. The file stays cached, so it opens offline next time.
  - **Writing** goes to a temp copy that is saved as a version when the app
    closes the file or calls `fsync`. A DAW holding a file open gets a
    version on each sync, and the write-a-temp-then-rename save lands as
    one version of the real name.
  - **Offline**, the manifest still browses and cached files open. A file
    that isn't cached fails with `ENETUNREACH` ("Network is unreachable").
- **Pinning from outside the app:** set `user.architect.pinned` to `1` or
  `0` on a root or anything in it (`setfattr -n user.architect.pinned -v 1
  <path>`), or run `architect-desktop files pin <path>` or
  `architect-desktop files unpin <path>`.
  `user.architect.state` reads `pinned`, `cached`, `fetching` or `online`.
  The app's Pin button and the xattrs show the same state.

It is the agent's: the node (`architect-desktop serve`) mounts it when its
config has a `drive` (`ARCHITECT_DRIVE`). The spec is in
[Project files](@/architecture/project-files.md); the engine under it is in
[Files](@/features/files.md).

## Running itself

`architect-desktop agent install` sets the device up. It writes the agent's
config if there isn't one, then installs a service that starts it at login
and restarts it if it dies.

| | Linux | macOS |
|---|---|---|
| Service | `~/.config/systemd/user/architect-agent.service` (`systemctl --user`) | `~/Library/LaunchAgents/com.fasttrackstudios.architect.agent.plist` (`launchctl`, `RunAtLoad`) |
| Config | `$XDG_CONFIG_HOME/architect/agent.toml` | `~/Library/Application Support/Architect/agent.toml` |
| Data (keys, documents, the blob cache) | `$XDG_DATA_HOME/architect/agent` | `~/Library/Application Support/Architect/agent` |
| Store (working copies of pinned roots) | `<data>/roots` (`files_dir`) | `<data>/roots` |
| Drive | `~/Architect` (`drive`) | `~/Architect` (macFUSE / FUSE-T); a File Provider later |
| Agent address | `127.0.0.1:18471` (`bind`), loopback only | the same |

To move the drive or the store, edit `drive` or `files_dir` in the config
and restart the agent, or use the app's **Settings → Drive**, which does
both (`DRIVE_CONFIGURE`). The same page, and a line on every project's
page, shows the drive's status: where it's mounted, what is syncing, offline,
and the cache's size (`FILES_DRIVE`). The other
commands are `agent uninstall` (its config and data stay), `agent start`
and `agent status`.

When the app opens, it finds the agent at the config's `bind`. If the
agent isn't running, the app starts the service, or, with no service, runs
`architect-desktop serve --config <agent.toml>` detached, so it outlives the
window. `ARCHITECT_FILES_AGENT` overrides the address. The unit's
`ExecStopPost` unmounts the drive however the agent ended.

## Unmounting, always

- **On a clean stop** (SIGTERM, the app's quit) the node unmounts.
- **After a crash or `kill -9`**, a guard process the drive started (the
  same binary, `drive-guard <mount>`) sees its pipe from the agent close
  and unmounts lazily. This is what fusermount's `auto_unmount` does, but
  without needing `user_allow_other` in `/etc/fuse.conf`.
- **At start**, a mount left in `Transport endpoint is not connected` (the
  guard was killed too) is cleared. A live one (another agent) is refused.

## Code tour

1. `features/files/src/drive/mod.rs` `mount`, `Mounted` (`unmount`, its
   drop), `guard_main`, `clear_stale`, `is_mounted`, `lazy_unmount`,
   `fusermount`.
2. `features/files/src/drive/fs.rs` `DriveFs`: the FUSE filesystem
   (`fuser`, no libfuse). Inodes are by mount path, and a rename moves a
   subtree. It passes through to pinned working copies, reads unpinned
   ones from the manifest, and stages writes in temp copies (`flush`,
   `fsync` and `release` save them).
3. `features/files/src/drive/cache.rs` `Cache`: the one place the drive
   fetches (`begin`, `read`, `copy_to`) and evicts (`evict`), so a
   holder's attestation has one hook.
4. `apps/desktop/src/node/files.rs` `mount_drive`, and
   `apps/desktop/src/node/cli.rs` (`files pin|unpin|status`, `drive-guard`).
5. `features/files/src/agent.rs` `Files::drive`: `DriveStatus` (mounted,
   syncing, offline, the cache's size).

## Tests to read

- `features/files/tests/drive.rs`: over a real mount, a project browses
  with its sizes, a take reads on demand and is then `cached`, a new file
  and an atomic save become versions on the node, pinning by xattr makes a
  working copy that the mount then passes through to, and unmounting
  leaves no mount.
- `apps/desktop/tests/drive.rs`: the real binary mounts; `kill -9` leaves
  no mount; a stale mount is cleared by the next start; SIGTERM unmounts.

## macOS

On a Mac the drive is meant to be a **File Provider**, the way Dropbox and
Google Drive work: it shows in Finder's sidebar under Locations, and lives
at `~/Library/CloudStorage/Architect-<domain>`. Until that ships:

- `architect-files` builds on macOS with fuser's `macos-no-mount`, so
  everything compiles without macFUSE.
- `drive::mount` says why it can't mount.
- The agent, its launchd install, sync and the WebDAV face all work, and
  Finder can mount WebDAV (Go → Connect to Server,
  `http://127.0.0.1:18471/dav/`).

**FUSE on a Mac in the meantime.** macFUSE or FUSE-T has to be installed
first. Both install as packages that need an admin password. macFUSE also
needs its kernel extension approved in System Settings → Privacy & Security
(then a restart); FUSE-T doesn't, because it runs over a local NFS server.
After that, build fuser without `macos-no-mount` (it links the installed
`libfuse`), and the same `DriveFs` mounts with `volname=Architect`.

**What the File Provider needs:**

- **Its shape.** An `NSFileProviderReplicatedExtension` (macOS 11+), as an
  app extension (`.appex`, extension point `com.apple.fileprovider-nonui`)
  inside an `Architect.app` bundle. The app registers its domain with
  `NSFileProviderManager.add(NSFileProviderDomain(identifier: "architect",
  displayName: "Architect"))`.
- **Talking to the agent.** The extension is Swift and talks to the agent
  over its HTTP face on `127.0.0.1:18471`; none of the Rust moves into the
  extension:
  - enumerating calls `FILES_ROOTS` and `FILES_BROWSE`;
  - fetching contents is `GET /dav/…`, which streams from a peer while the
    bytes arrive;
  - create and modify are `PUT /dav/…`, delete is `DELETE`, rename is
    `MOVE`;
  - Finder's "Keep Downloaded" (`contentPolicy`,
    `.downloadEagerlyAndKeepDownloaded`) maps to `FILES_PIN`, "Remove
    Download" to `FILES_UNPIN`;
  - changes reach Finder through `signalEnumerator` when the manifests
    change.
- **Entitlements.**
  - Both bundles: an App Group shared by the app and the extension
    (`com.apple.security.application-groups`,
    `group.<TEAMID>.com.fasttrackstudios.architect`).
  - The extension: the App Sandbox (`com.apple.security.app-sandbox`) and
    network client (`com.apple.security.network.client`), to reach the
    agent on loopback.
- **Signing.** Two App IDs (the app, and the extension under it), both with
  the App Group capability, and a provisioning profile for each.
  - **Development:** an Apple Development identity (airlock has one, team
    `T79FBHC959`) with profiles made by `xcodebuild
    -allowProvisioningUpdates`. That needs Xcode signed in to the team, or
    an App Store Connect API key (key id, issuer id, `.p8`) on the build
    machine.
  - **Shipping outside the App Store:** a Developer ID Application
    certificate, and notarization (`notarytool` with the same API key).
- **From the team before it can be built:**
  - the team and bundle ids;
  - Xcode signed in on the Mac that builds, or an App Store Connect API
    key there;
  - a Developer ID certificate, for distribution.

## Status

- **Linux:** done and checked live (mount, on-demand open, pin by xattr,
  edit through the mount, `rsync`, crash safety under systemd).
- **macOS:** the agent, launchd and WebDAV work. The FUSE drive needs
  macFUSE or FUSE-T installed and fuser rebuilt. The File Provider is
  specified above and needs the signing setup.
