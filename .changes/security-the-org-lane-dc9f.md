---
"architect-desktop" = "patch"
---

Security: the org lane gives up on a call to the issuer (`whoami`, the org list) after `ISSUER_TIMEOUT` (10 s, or the re-check interval if shorter; `IssuerLane::with_timeout`) and treats it as unreachable: an issuer that stalls no longer keeps a revoked live note open past the grace, and a new call is refused instead of hanging.
