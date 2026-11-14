---
"architect-app" = "patch"
---

Fixed opening a vault note by path (an extension's `open`, `EXT_OPEN {path}`, a notification's go to) once the navigator's tab was hidden, and on the phone layout: the opener now lives in `VaultPage` (#739, #725)
