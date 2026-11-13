---
"architect-desktop" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
---

Added ShareGuest (features/spaces guest): the read-only host service of a share link's guests at POST /s/<code>/_guest/{link,read,files}, checked by the same Shares::open and admit as every page request; ShareUrl::parse for opening links safely; architect_sites::plain (a page's prose as plain blocks, Options::page_data writes _note.json); and /.well-known/apple-app-site-association from the node's new ARCHITECT_APPLE_APP_IDS (#629)
