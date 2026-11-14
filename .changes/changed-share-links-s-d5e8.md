---
"architect-desktop" = "minor"
"architect-spaces" = "minor"
---

Changed share links: `/s/<code>/` and every page path below it (and `<link>_files/`) answer the web app's own page from the node (`architect_desktop::node::share_shell`: a `<head>` from the link's title, the app's Content-Security-Policy), behind the same `Shares::open` and `admit` checks; the node no longer serves a link's built HTML pages, files bar, queue, comment form or "Being built" page (refusals are plain text). Added `ListedNote::art`, and the guest service takes the node's sign-in cookie for an empty token on its own origin. A share with a domain of its own stays a Site (#646).
