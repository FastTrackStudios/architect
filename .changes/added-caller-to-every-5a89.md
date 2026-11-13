---
"architect-extensions" = "minor"
---

Added `caller` to every action context (linked, process, WASI, web): the calling extension id from the call principal (`invocation::caller_json`), so a service can tell whose call it is.
