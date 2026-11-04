---
"architect-a2ui" = "patch"
---

Fixed `architect-a2ui` (default build and feature `ui`) for `wasm32-unknown-unknown`: `Surfaces` holds its `CrdtDoc` so the Action List handlers stay `Send` in the browser, and `Surfaces::doc` is no longer a `const fn`
