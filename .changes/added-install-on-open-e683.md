---
"architect-app" = "minor"
"architect-extensions" = "minor"
---

Added install on open: a vault or project folder names its extensions in `.architect/extensions.toml` and can carry signed archives in `.architect/extensions/` (`vault_needs`, `Extensions::open_vault`, `Extensions::set_source`, `EXT_NEEDS`); the app shows what it needs over the Vault with "Install all" through one consent prompt (`VaultNeedsBanner`), and `MissingExtensions` lists what cannot be had with why (`on_done`, `Missing` is `Display`)
