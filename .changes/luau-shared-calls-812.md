---
"architect-scripting" = "minor"
"architect-extensions" = "minor"
---

Added one implementation of a Luau extension's globals apart from Luau (#812): `architect_extensions::luau_calls` (`host_request`, `raw_request`, `ext_call`, `HOST_FUNCTIONS`, `EXT_FUNCTIONS`, `lenient`) turns a `host.*` or `ext.*` call's arguments, as JSON, into its host request or contribution change, and `architect_scripting::bindings` (new feature `bindings`, wasm-clean) does what `actions.*` and `state.query` do. The desk's mlua `host` and `ext` modules now call them, and a browser's Luau runner calls the same code. `architect_extensions::host::json_args` is public. mlua and Tokio are native-only dependencies of `architect-scripting`, so its `luau` feature is `bindings` alone on wasm.
