---
"architect-extensions" = "none"
"architect-sandbox-board" = "minor"
"architect-app" = "minor"
"architect-pages" = "minor"
"architect-scripting" = "minor"
---

Added A2UI JSON pages (`<view>.a2ui.json`, `Pages::with_a2ui`, feature `a2ui`: components, data and `bind` to an extension's actions, drawn through the A2UI catalog), workers and timers for Luau pages (`worker.spawn("x.luau")` with `w:on_message`/`on_progress`/`on_done`, `set_timeout`/`set_interval`; `architect_scripting::worker::Workers::install_owner` and `deliver`), `cancel()` on a TSX page's worker, and source maps for TSX pages: a runtime error says the `.tsx` file's own line and column in `<file>.error`, `APP_STATUS` and `EXT_CHECK` (`tsx::transpile_mapped`, `SourceMap`, `remap`)
