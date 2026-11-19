---
"architect-web" = "minor"
"architect-desktop" = "patch"
---

Added `NOTE_OPEN {path}` to what a frame's view may ask of the web app: the note opens only where the viewer may already open it (a share link's guest: a note the link lists), and reads nothing. A share link's guest is shown no control that needs a vault or a write (no Files, Open, New note, Vault, Search or navigator drawer); signed in, they are back (#910). The node sends an island's scripts, modules, sheets and wasm under `/_ext/` gzipped to a browser that takes it, and a note's view is handed its own files and those of the notes it lists, not every file of the link (#908, #602).
