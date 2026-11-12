---
"architect-app" = "minor"
"architect-sandbox-media" = "minor"
"architect-shell" = "minor"
"architect-workspace" = "minor"
---

Added the navigator to a phone's left drawer: `WorkspaceView`'s new `drawer_dock` puts the left dock in the shell's drawer, under the app's own side (the Architect app's org vaults); the drawer opens by an edge swipe, the action bar's Sidebar button or `SHELL_SIDEBAR_TOGGLE`, and a note opened from it puts it away. Added `architect_shell::drawer_open` and `LAB_STATE`'s `drawer`.
