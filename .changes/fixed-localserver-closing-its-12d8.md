---
"architect" = "patch"
---

Fixed `LocalServer`: closing its `Scope` now shuts each in-process session down, so the client's `Caller::closed()` resolves (and `use_connect_supervised` reconnects) as it would when a remote server dies; before, the session outlived its server
