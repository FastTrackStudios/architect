---
"architect-navigator-api" = "none"
"architect-workspace" = "minor"
---

Changed scope tabs (decks) to draw in the normal tab bar (#476): `ScopeTabs` leads the main window's first tab bar, then a divider, then the scope's own tabs, nothing with none open; removed `DeckStrip`, `WorkspaceView`'s `under_strip` and `DecksConfig::on_new`; added `DecksConfig::general`, `use_tab_kinds` with `TabKindsUi`, `TabRender` and `TabRenderCtx` (a kind's own drawing), `DeckHandle::show_general` and `set_colour`; a custom tab kind's panes draw their tab its way.
