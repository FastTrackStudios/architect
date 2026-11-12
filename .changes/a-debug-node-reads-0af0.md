---
"architect-sites" = "minor"
---

A debug node reads the share pages' site.css, site.js and player.js from ARCHITECT_SITES_ASSETS_DIR at every build of a site (Studio's local stack: a player edit shows on reload, no relink); `site_js` answers a `Cow<'static, str>` and `site_css` is new. `just dev`, `just demo`, `just website` run the local stack (Studio's, with this Architect, when ../studio is beside the checkout).
