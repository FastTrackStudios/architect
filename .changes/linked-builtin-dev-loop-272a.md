---
"architect-extensions" = "minor"
"architect-lab-board" = "none"
"architect-lab-ui" = "none"
"xtask" = "none"
---

EXT_CLONE of a linked built-in with a `[source]` writes its crate to the development folder and builds it as a process that keeps its id and overrides the built-in live, rebuilt on save (the last good build kept, a failed build in APP_STATUS and EXT_LOGS); `cargo xtask builtins` builds each built-in crate linked and as a process; a benchmark of each runner.
