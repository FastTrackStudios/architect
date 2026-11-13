---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-favourites" = "minor"
"architect-shell" = "minor"
"architect-web" = "minor"
---

Added favourites that follow the open note's type in the app's shell on desktop, phone and browser: `architect_shell::declare` turns an extension's `[[favourites]]` into the bar's tabs (each target read by the app, a tab it cannot open left out, the first four kept, the primary's second face while its state is on), `Favourites::press` runs a declared tab, `declared_tab_id` names it, `Tab::named` and `Action::glyph` draw a Lucide icon by name, and `resolve_favourites` now returns an owned `Favourites` that keeps the default's round button when a type declares none. The Architect app reads the loaded extensions' declarations for the Vault's note on show (`page`, `search`, `<extension id>/<view>`, actions it has, a `place` naming a note beside it); the browser app does the same from the node's new read-only `GET /_ext/favourites.json` (`FAVOURITES_LISTING`), opening an island's view in a frame of its own. Added `architect_favourites::place_path`.
