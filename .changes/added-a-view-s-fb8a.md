---
"architect-extensions" = "minor"
"architect-workspace" = "minor"
---

Added a view's own place that survives its program restarting: `linked::use_view_state()` (`ViewState`, `ViewState::set`) gives a view, linked in or in a process, the string its pane keeps for it (zoom, scroll, selection, a mode), back when a rebuilt development copy restarts or the pane moves or pops out. New `Pane::view_state`, `PaneHandle::view_state`/`set_view_state`, `TabSetArgs::view_state` (`WORKSPACE_TAB_SET`), host call `set_view_state`, `remote::offer_view_state`; `linked::pane_input` takes the view state
