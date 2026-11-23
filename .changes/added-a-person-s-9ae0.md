---
"architect-extensions" = "minor"
"vault-live" = "minor"
---

Added a person's installed extensions on all their devices: `person_sync` (`.architect/people/<id>/extensions.toml`, `PersonInstalls`), `Extensions::publish_installs` and `apply_installs` (`EXT_INSTALLS_PUBLISH`, `EXT_INSTALLS_APPLY`; the archive pinned and signed by a key the device already trusts, grants given, no prompt), `Installs::archive_of`, `install_bytes`, `trust_with`; the sync client's index keeps the vault's own `.architect/` folder (`vault_sync_client::SYNCED_DOT`)
