---
"architect-desktop" = "minor"
"architect-spaces" = "minor"
---

Added `ARCHITECT_WEB_APP` (`Config::web_app`, `Shares::with_web_app`): a share link's page is the web app's only when the web app is served beside the node, otherwise a plain 503 saying the link needs it; the page carries `architect_spaces::guest::SHARE_SHELL` and the node's version. Fixed: a link's own HTML, SVG, XML or script is a sandboxed download (`Content-Security-Policy: sandbox; default-src 'none'`, `attachment`), an unknown code answers byte for byte as a revoked or expired one, and sign-in/sign-out's `to` refuses control characters and backslashes (#646).
