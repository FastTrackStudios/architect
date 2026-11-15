---
"architect-extensions" = "minor"
"architect-pages" = "minor"
"architect-sandbox-web" = "minor"
---

Added TypeScript actions in a browser (#536): `WebPages::run_action` runs a `typescript` extension's action in a Web Worker on the browser's engine (the page worker's action mode, the same prelude and bridge, its host calls answered by its `WebHost`), and `WebPages::register_actions` puts a manifest's TypeScript actions on an Action List; no QuickJS, no JSPI. `architect_extensions::caller_json` and `invocation_json` are public. The web lab runs the showcase's Recipe scaler (`?page=recipes`).
