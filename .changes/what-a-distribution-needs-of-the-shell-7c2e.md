---
"architect-workspace" = "minor"
"architect-commands" = "minor"
"architect-shell" = "minor"
"architect-extensions" = "minor"
"architect-app" = "minor"
"architect-mcp" = "patch"
---

Added what a distribution's own layout needs of the shell: a view that fills the title slot (`Slot::Title`) gets the title row's place, clear of the window's buttons, and the tab bars under it are ordinary ones; an extension's view fills its pane with no header row, and no tab bar while it is alone in its group (its toolbars keep a row); a pop-out window off a Mac has no system decorations and draws its own buttons (`architect_workspace::host::PopoutButtons`); `APP_SHOT` takes `window` (a pop-out's id from `WORKSPACE_LAYOUT`) and finds a pane's window from the pane (`shot::use_window_shot`); a plain key the keymap binds everywhere (Space for play) runs wherever the focus is unless a field is typed in (`Commands::plain_key_everywhere`, `Commands::run_plain_once`); a linked view's layers read an engine's values (`PaintMount::with_engine`), from the shared table of the extension providing a service it uses; the Architect app registers the `SCRIPT_*` actions. Fixed `architect_mcp::parity::Parity` calling an answer MCP cut at `MAX_TEXT` a disagreement with the socket's whole one.
