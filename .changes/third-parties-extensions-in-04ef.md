---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-extensions" = "minor"
---

Third parties' extensions in the Architect app: the store's registry and registries a person adds in Settings → Extensions (git or folder, kept in settings), each trusting its publishers' own keys, so a publisher signs its .archext with its own key; EXT_REGISTRIES, EXT_REGISTRY_ADD/REMOVE/REFRESH; development installs: EXT_DEV/EXT_DEV_END/EXT_DEV_INSTALLS and `architect-desktop extensions dev <folder>`, behind a setting or a debug build (#493).
