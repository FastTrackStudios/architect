---
"architect-extensions" = "minor"
---

Added the npm package @fasttrackstudio/architect (features/extensions/web, published on a v* tag once NPM_TOKEN is set): loadExtension runs an extension's WASI component in any web app with deny-by-default capabilities (the manifest's [capabilities] within the embedder's grant, checked as hostcall::check checks them), a confirm hook for destructive actions, a typed handler per HostRequest op, memoryHost, mountView with the share pages' sanitiser rules, ArchitectView for React, and architect build-web (transpile, architect.json with the world version, typed actions, a size report). Fixed WASI components in a browser: every host import that waits for the page (notify, pick, documents, fetch, …) is now JSPI-async, so it answers instead of trapping or dropping its error, and the editor and port imports are linked.
