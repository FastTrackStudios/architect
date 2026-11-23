---
"architect-extension-archive" = "minor"
"architect-extensions" = "minor"
"vault-live" = "minor"
---

Changed install sync after review: a person's record is signed by the device that wrote it (`DeviceKeys`, feature `person-sync`) and applies only on a device that trusts it as theirs; installs, removals and revokes sync (`at_ms`, `removed`, `Extensions::sync_person`, `on_installs_changed`, `Extensions::revoke`); grants only after an install; `EXT_INSTALLS_*` act for the signed-in person only; `ExtensionPaths` keeps `.architect/people/<id>/` the person's; `verify_detached` in the archive crate
