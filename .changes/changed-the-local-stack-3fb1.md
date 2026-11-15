---
"xtask" = "none"
---

Changed the local stack: `devenv up` runs the app, which is also the node, and the web app; no separate node process (`cargo xtask dev run app --headless` is the same app's `serve`, for the stack's test).
