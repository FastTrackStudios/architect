---
"architect-desktop" = "minor"
"architect-extensions" = "minor"
"vault-live" = "minor"
---

Added `vault_live::ExtensionPaths`, the node's write guard: only a share's owner writes its `<code>.extensions.toml` and packages; share needs are read and written without following a link, their size checked first; "Install all" previews and installs off the window's thread
