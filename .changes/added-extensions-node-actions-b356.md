---
"architect-accounts" = "minor"
"architect-app" = "minor"
"architect-desktop" = "minor"
---

Added extensions' node actions on the node (#583): `architect-desktop serve` serves installed extensions' `NodeActions` on its org lane (`OrgLane::with_node_actions`), with operator secrets from `ARCHITECT_EXTENSION_SECRETS`; the app runs them through the account's org lane (`Accounts::org_lane`, `OrgNode`)
