---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-extensions" = "minor"
"architect-webview" = "minor"
"architect-workspace" = "minor"
---

The web view kind (#222): an extension ships a TypeScript/npm app as a kind = "web" view (src in its package), drawn in a wry child web view clipped to its pane on a desk (Linux under X11, or Wayland with ARCHITECT_WEBVIEW_X11=1) and a sandboxed iframe in a browser (@fasttrackstudio/architect/webview-host), reaching the app through a bridge whose every call is checked against its capabilities; store_secret keeps a secret insert-only; Host::hear passes events to a view; architect-desktop feature webview.
