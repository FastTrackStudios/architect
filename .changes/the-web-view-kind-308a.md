---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-webview" = "minor"
"architect-workspace" = "minor"
---

The web view kind on a desk (#222): architect-webview (feature desk) draws an extension's kind = "web" view in a wry child web view clipped to its pane, following splits, floats, pop-outs and tabs, hidden under the app's overlays (LiveDocument::covered_at); architect-app/architect-desktop feature webview; ARCHITECT_WEBVIEW_X11=1 runs the window under XWayland on Linux; pop-outs carry WebViews, ViewPages and Meters.
