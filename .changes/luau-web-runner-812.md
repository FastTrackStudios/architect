---
"xtask" = "none"
---

Added Luau extensions in the web app (#812): `architect_pages::web::luau::WebLuauRunner` runs each Luau action and hook in a Web Worker on luau-web's Luau (vendored in `apps/web/assets/luau-web` by `scripts/vendor-luau-web.sh`, fetched only when a Luau extension runs), every global it calls answered on the page through the host service (`WebHost::through`) by `architect_extensions::luau_calls` and `architect_scripting::bindings`, the code a desk's runtime uses. Its limits are a TypeScript action's. `WebPages::limits` is public. The web app's status line shows extensions' status items.
