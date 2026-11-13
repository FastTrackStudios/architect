---
"architect-command-palette" = "minor"
---

New feature `extension` (the crate's default) holds the `Extension` value and its process copy; without it the crate is its views (`views::PaletteView`, `QuickOpenView`) for an app with no extension host. The workspace now pins the crate with `default-features = false`: name `extension` to keep the extension.
