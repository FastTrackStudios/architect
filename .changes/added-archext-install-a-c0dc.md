---
"architect-extension-archive" = "minor"
---

Added `archext install`, a binary that checks a signed extension archive against a pinned sha256, id, version and publisher keys (the same `open` an app runs) and unpacks it: what the flake's `lib.architectExtension` and `nixosModules.architect-node` (`services.architect-node.extensions`) build with (#583)
