---
"architect-app" = "minor"
"architect-command-palette" = "minor"
"architect-commands" = "major"
"architect-extensions" = "minor"
"architect-launcher-core" = "minor"
"architect-palette" = "minor"
---

Changed ⌘K and ⌘O into an extension (#381). Added the palette engine `architect-palette` (MIT, no UI: `Engine`, `Source`, `FnSource`, `Ranking`, `Item`, `rank`, `rank_open`) and the Command Palette, a linked built-in extension (`architect-command-palette`, id `command-palette`) that draws both overlays in the shell's palette slot (`CommandLayer`), binds ⌘K, ⇧⌘P and ⌘O, and provides the `architect.palette` service (`COMMAND_PALETTE_SHOW`, `COMMAND_PALETTE_SEARCH`). Extensions get three manifest points: `[[palette_views]]` (which view draws an overlay), `[[keys]]` (a key bound to an action, now read) and `[[palette_sources]]` (rows for ⌘K, ranked by the engine); `EXT_PALETTE` lists them. `COMMANDS_SHOW` fails when no palette draws the overlay, and `COMMANDS_STATE` names the extension that draws each one. The launcher ranks with the engine (`ProviderSource`).

Migration: `Commands::handle_chord` no longer opens the palette on ⌘K/⌘O: link `architect_command_palette::CommandPalette` (or set `Commands::set_palette_views` and `Commands::set_bound_keys` yourself). `CommandPalette` and `QuickOpen` left `architect-commands`; draw with `architect_commands::kit` (`Frame`, `CommandRow`, `Lit`, `AskPalette`, `Pick`) over `Commands::palette_rows`, `Commands::pick`, `Commands::open_rows` and `Commands::open_found`. `Ranked::command` is `Ranked::item`. `fuzzy`, `rank`, `Recents`, `Openers` and `Command` (an `Item`) now come from `architect-palette` and are still re-exported by `architect-commands`. A manifest's `[[keys]]` moved from `Manifest::keys` to `Contribution::key_bindings`. The lab's `LAB_SHOW_COMMANDS` and `LAB_OPEN_ANYTHING` are the extension's `COMMAND_PALETTE_OPEN` and `COMMAND_PALETTE_OPEN_ANYTHING`.
