---
"architect-shell" = "minor"
---

Added an island's home view (`SiteIsland::home(view)`): signed in on an org's vaults, the web app opens on it, handed the vault's typed notes as rows (`{path, open, type, properties}`, never a note's text) and what was opened lately; `NOTE_OPEN` from it opens the note in the workspace. `SiteIsland::page_view(view)` lists the views that fill a page, so the app opens only a view it has. A note its type's view draws has its type's tabs at the bottom for a member too, a declared menu tab opening above itself, and a back button. `architect_shell::quiet_actions(quiet)` puts a phone's row of buttons over its bottom tabs away on a page for looking and listening (`ACTIONS_QUIET`). `PLAYHEAD_DETAIL` opens the Now Playing bar's detail (`architect_playhead::ui::toggle_detail`, `DETAIL_OPEN`).
