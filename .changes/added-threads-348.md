---
"architect-threads" = "minor"
"architect-workspace" = "minor"
"architect-extensions" = "minor"
"architect-app" = "none"
---

Added Threads (`architect-threads`, a built-in extension, id `threads`): conversations anchored to any entity (`entity_type` + `entity_id`, a note by its path) with comments, replies, `@name` mentions and resolve/reopen, kept as notes in `Threads/` so they sync between devices and through the headless node like any note. The comments pane (`THREADS_PANE`) follows the open note; open threads show as a badge on the note's tab. Actions `THREADS_LIST`, `THREADS_GET`, `THREADS_START`, `THREADS_POST`, `THREADS_REPLY`, `THREADS_EDIT`, `THREADS_DELETE_COMMENT`, `THREADS_RESOLVE`, `THREADS_REOPEN`, `THREADS_DELETE`, `THREADS_COUNTS`, `THREADS_MENTIONS`, `THREADS_REVEAL`; service `architect.threads`. `WorkspaceStatus` gained `focused_view` and `focused_state` (the focused pane's kind and what it shows), and the app's selection passes them to extensions. Added `Host::set_path_opener`: a path no extension's opener claims opens with the app's own (the Architect app opens a vault note), so `Host::open` and an extension's `open` reach notes; and `EXT_VIEW_CLOSE` with `Host::close_view`/`Host::set_view_closer`, so an extension's popup or overlay closes itself.
