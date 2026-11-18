---
"architect-extensions" = "minor"
---

Changed a manifest rule: an extension may [uses] a service it provides itself, since another provider can be chosen to answer it (EXT_SERVICE_USE); Keyflow hosts fts.session and follows the daw engine's session. [requires] of its own service is still refused
