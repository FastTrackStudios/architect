---
"architect" = "major"
"architect-control" = "minor"
"architect-workspace" = "major"
"architect-commands" = "minor"
"architect-extensions" = "minor"
"architect-theme" = "minor"
"architect-desktop" = "minor"
"architect-app" = "minor"
"architect-sandbox-clicker" = "none"
---

Added the agent tools for a running app (#268): the Action List refuses an unknown argument name with the valid ones (every door), carries a `Problems` registry and a `StatusBoard` (`ActionList::problems`, `ActionList::status`) read by `APP_STATUS` (`architect::action::app_status::register`); `architect_control::guide::Guide` writes `<app dir>/AGENTS.md` and `actions.schema.json` from the live list; `actions shot [file] [--pane id]` runs `APP_SHOT` (`architect_workspace::shot::use_app_shot`, feature `native`: the window's Blitz document painted on the CPU); `EXT_CHECK {folder}` (`architect_extensions::check::Checker`, `Host::checker`) and `EXT_CLONE {id, as?}` (`Extensions::with_dev_root`, `clone_to_dev`), `Extensions::problems` / `report_problems`; `WORKSPACES_CLONE {name, as?}` and `WORKSPACES_RESTORE_BUILTINS`; the escape hatch Ctrl+Alt+Shift+Escape (Cmd on macOS, `Commands::escape_hatch`, `is_escape_hatch`), which `Keymap::bind` refuses; the theme reports its `theme` section and problems (`architect_theme::actions::report`); the lab writes its guide, serves `APP_STATUS`/`APP_SHOT`, offers `EXT_*` on its list, adds `LAB_GO {tab}` and `architect-desktop extensions check <folder>`.

Migration: an action call that sent an argument name the action does not take (ignored until now) is refused with `bad_args`; send only the names its schema lists. `WORKSPACES_SAVE` over a built-in screenset from a caller with a principal (MCP, the CLI, a script run as someone) answers `changed: false, reason: builtin`: `WORKSPACES_CLONE` it first. `control::Env` has a new field `shipped` and `PaneInfo` a new field `rect` (struct literals add them; `Env::new` sets `shipped` empty). `Package::embedded` keeps the manifest's text among its files as `extension.toml`.
