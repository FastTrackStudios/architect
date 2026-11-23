---
"architect-extensions" = "minor"
---

Added EXT_SERVICES and EXT_SERVICE_USE: when several loaded extensions provide one service (fts.session from a DAW engine and from Keyflow), a person or a launcher chooses which one answers (Host::use_provider, Host::service_providers); of providers at the same version the host now takes the first by id (interop::choose_provider) instead of whichever sorted last
