---
"architect-extensions" = "minor"
"architect-extension-wasi" = "minor"
"architect-sandbox-wasi-counter" = "none"
"architect-extension-wasi-hazards" = "none"
"architect-app" = "none"
---

WASI components get workers: more instances of a component on a pool thread, each with its own memory and time limits, spoken to by messages (`worker` import, `work` export), with back-pressure, cancellation when the owner goes, lazy start and idle stop. The lab's WASI Counter counts primes in one. `Host::forward_actions_to` puts every extension's actions on an app's own list, so the lab's control socket runs them. The world gained exports: a component built against the old one no longer instantiates (pre-1.0).
