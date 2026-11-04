---
"architect-theme" = "minor"
"architect-settings" = "minor"
---

Added theme persistence and a standard Appearance section (#185). The person's choice and token overrides are kept as synced settings, this device's as device settings, and the org's default comes from its settings. Each one loads at start and is written, debounced, when it changes (`architect_settings::appearance`). The Appearance page covers mode, theme cards drawn in their own colours, text size, density, corners, contrast and font, plus "reset to organization default". `THEME_SET` now writes the person's layer unless `layer` says otherwise (breaking: it used to write `pick`). New: `ThemeStore::pick_in`, `composed_through`, `layers`, `subscribe_layers`, and the `text-scale`, `density` and `contrast` tokens.
