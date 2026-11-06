---
"architect-shell" = "minor"
"architect-workspace" = "minor"
"architect-app" = "minor"
"architect-sandbox-board" = "minor"
---

A desk's status bar is now the page's foot: it starts where the left side (the ribbon, or the sidebar if it is open) ends and stops at the right side, so the sidebars run to the window's bottom. The lab's Board no longer fills the status slot with a bar of its own; its pin count is a status item in the shell's bar. Added `AppShell`'s `foot`: the app's buttons above the workspace switcher and the account (the lab's Theme and Settings), shown as rows in a docked sidebar and as icons in a folded sidebar's ribbon and in a rail. Added `WindowChrome::framed` and `WindowControls`: off a Mac, the window has no system decorations and the shell draws close, minimize and maximize as traffic lights at the title row's left in a 40pt row, so a compositor that draws server-side title bars adds no strip. Added `MainWindow::close`. Tab headers are now Obsidian's: 40pt, the current tab 34pt with a three-sided outline, open into its pane, its corners curving into the bar's rule (`TabFlares`), short dividers between the other tabs, and no accent stripe. The page's own tab in the title row is drawn the same way. While a sidebar folds or opens, the title row's tabs follow it and never pass under the window's buttons.
