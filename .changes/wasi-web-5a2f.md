---
"architect-extensions" = "minor"
"architect-web" = "none"
---

WASI component extensions run in a browser (feature `web`, wasm32-unknown-unknown): the same components, transpiled by jco, each instance in a Web Worker. `WebHost` answers their host imports with the same capability checks as the native runner (`hostcall::check`, now shared by `hostapi`); `WebComponent` starts one, puts its actions on the app's list and mounts its views in the page's DOM. Limits: memory capped in the module, a call's time by terminating its Web Worker. Workers are Web Workers, with the native queue sizes and failure semantics. The lab's WASI Counter runs in a browser page (`apps/lab/web`, `just lab-web`). `host::invocation_json` moved to a module of its own (re-exported where it was).
