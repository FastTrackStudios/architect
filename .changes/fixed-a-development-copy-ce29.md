---
"architect-extensions" = "patch"
---

Fixed a development copy whose package holds a large program stalling the window: the watcher knows files of 1 MiB and more by their size and modification time instead of hashing them on the window's thread every poll.
