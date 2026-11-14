---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-mobile" = "minor"
---

Added deep links into actions (#186): `architect://<route>?<args>` and the universal link `https://<domain>/a/<route>` run a declared route's action (`architect_app::links`: `Route`, `ROUTES`, `declare`, `follow`), never an action id the link names; the app's own are `capture` (`CAPTURE_OPEN`) and `capture/add` (`CAPTURE_ADD`). A link runs as `Principal::Service { name: "link" }` asking the person before any write, and never runs a destructive action. The iOS scene and a desk's launch arguments and second-launch hand-off route them; the node's apple-app-site-association claims `/a/*`
