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
(or use the app's Drive settings), then restart the agent. The other
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

## Status

Linux works. macOS: the same code over macFUSE or FUSE-T, and a File
Provider, are the next layer (`skills/architect/references/files.md`, the
drive).
