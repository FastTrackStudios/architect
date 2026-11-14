---
"architect-extensions" = "minor"
"architect-workspace" = "minor"
---

The web view kind (#222): an extension ships a TypeScript/npm app as a kind = "web" view (src in its package); architect_extensions::webview serves the package under a CSP with no network and bridges the view to the host, every call checked by hostcall::check; store_secret keeps a secret insert-only; Host::hear passes events to a view; KindSpec::web; the npm package gains @fasttrackstudio/architect/webview (typed SDK) and /webview-host (a sandboxed iframe host).
