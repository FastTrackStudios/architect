---
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-sites" = "minor"
---

Islands (studio#445): a distribution links in a WASI component whose views draw live on a share link’s now-playing view (Distribution::island, architect_sites::SiteIsland); the node serves its files, the web runtime and a sandbox document from /_ext/, the sandbox its own document with its own CSP (sandbox allow-scripts, no network), so the share page’s CSP is unchanged; the player brings a view in through the runtime’s share-page allow-lists, keeps /S/… apart from /s/…, and follows a chart a third of the way down.
