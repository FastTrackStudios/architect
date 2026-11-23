---
"architect-extensions" = "minor"
---

Changed install on open after review: a vault's carried archives are a source for its own plan only (never an update, search or other install), signed by the org key or the installed copy's signer, read with `peek` under 16 MB and cached, and no link is followed; installs record their signer and refuse an update by another key (`InstallError::SignerChanged`); a project never downgrades or steps past a pin (`Missing::WantsOlder`, `Missing::OutsidePin`, `InstallError::Downgrade`, `EXT_INSTALL {explicit}`); `Extensions::accept` takes a grant and installs nothing without one, `accept_vault` and `previews_for` drive "Install all"; `set_source` is removed; the banner plans off the window's thread
