---
"architect-navigator" = "minor"
"architect-app" = "none"
"architect-settings" = "patch"
"architect-commands" = "patch"
---

Added `architect-navigator` (`features/navigator`), a port of Notebook Navigator as the sidebar: `NavigatorSource` and `MemorySource`, a folder tree beside a virtualized list of notes (dual or single pane, resizable, auto-reveal, collapse or expand all, back and forward, sort and date groups, multi-select, keys across both panes), `NAVIGATOR_*` commands, the `navigator` settings section and `navigator.*` theme roles. It is GPL-3.0-or-later, the one GPL crate: an app that links it ships under the GPL. The lab's Vault sidebar is the navigator. The settings page draws a section's `folder-tree` icon. Fixed `use_command` panicking when its component outlives the commands provider as a window is torn down.
