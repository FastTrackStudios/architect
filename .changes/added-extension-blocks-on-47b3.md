---
"architect-desktop" = "minor"
"architect-extension-wasi" = "minor"
"architect-sandbox-wasi-counter" = "minor"
"architect-sites" = "minor"
---

Added extension blocks on the node's share links and Sites (#488): `architect_sites::InstalledBlocks` draws the fences of the extensions installed in `ARCHITECT_EXTENSIONS` (`<data dir>/extensions`), embed pages and, with feature `wasi` (`architect-desktop` `node-wasi`), a WASI component's embed view (`ComponentBlocks`; the guest reads its input with `architect_extension_wasi::embed()`), ahead of a distribution's or an embedder's `Blocks`; `Blocks` is a `BlockRenderer` (`claims`, `drawn_by`). Security: a renderer's markup (`Drawn::Html`) is now sanitised (`architect_sites::sanitize`): inline SVG keeps its geometry, classes and `data-*`, and loses scripts, `on…` handlers, `javascript:` URLs and `<foreignObject>`. The share player follows SVG `[data-at]` elements, scrolls the nearest scrolling box by bounding rect and fills the current SVG bar from the theme.
