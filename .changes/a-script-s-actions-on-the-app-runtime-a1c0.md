---
"architect-scripting" = "patch"
"architect-extensions" = "patch"
---

A Luau script's action calls run on the app's Tokio runtime again (the one its run was started from, Runtime::with_runtime or with_dispatch), awaited over a channel from the script's thread: an action that needs a runtime (a timer, a socket, tokio::spawn) no longer fails when a script calls it. `architect_scripting::vm` is public (the pool, Guard, Wake, Dispatch) for other script runners.
