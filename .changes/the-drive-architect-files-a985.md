---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-files" = "minor"
---

The drive (architect-files feature drive): every root a device knows mounted as a real folder over FUSE, pinned roots passed through to their working copy, unpinned ones listed from the manifest, fetched on open while the bytes arrive and kept, writes saved as versions; pin by xattr or architect-desktop files pin; a guard unmounts after a crash and a stale mount is cleared at start. The node mounts it with drive (ARCHITECT_DRIVE); Files::drive reports its status.
