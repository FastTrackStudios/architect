---
"architect-commands" = "minor"
"architect-extensions" = "minor"
"architect-shell" = "minor"
"architect-workspace" = "minor"
---

dev --collapse left,right (ARCHITECT_DEV_COLLAPSE) opens an instance with those sides folded away, and dev --hide rail,status,left,right,bottom or all (ARCHITECT_DEV_HIDE) takes them away, leaving the tab bar and the open view; running dev again with another --only turns back on what it names. The Vault opens its sidebar folded to the rail when no loaded extension fills the left dock (ExtensionUi::fills_dock), and no longer opens the Search placeholder in its left dock; a dock's icon bar with one view shows no tab. The right dock has a toggle at the title row's right end, as the sidebar's at its left, and folded, its rail sits under the title row, which runs across the window. With the ribbon hidden, the title row keeps clear of the window's and the sidebar's buttons. New commands SHELL_RAIL_TOGGLE and SHELL_STATUS_TOGGLE (⌘K, and the CLI through COMMANDS_RUN); a right press on the sidebar's button or the right sidebar's opens a menu of the rail, the status bar, the right sidebar and the sidebar's layouts (architect_shell::use_side_menu). The right sidebar's button sits at its inner edge: at the start of its tab bar while it is open, beside its rail while it is folded.
