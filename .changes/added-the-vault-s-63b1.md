---
"architect-desktop" = "minor"
"architect-web" = "minor"
"vault-proto" = "minor"
---

Added the vault's actions for signed-in web apps (#495 H3): the org lane serves `VaultActions` (`vault_proto::vault_actions`: `actions`, `run_read`, `run_write`, gated by the member's read or write on the vaults), the desktop app's own `VAULT_*` over each vault's folder; the web app proxies them for its linked extensions (an extension's call needs `reads`/`writes = ["documents"]`), names documents inside the org's vault as on a desk, opens a path an extension names, and in the demo answers that they need a sign-in.
