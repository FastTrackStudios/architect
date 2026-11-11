---
"architect-desktop" = "minor"
"architect-extension-wasi" = "minor"
"architect-sandbox-wasi-counter" = "minor"
"architect-sites" = "minor"
---

Added extension blocks on the node's share links and Sites (#488): `architect_sites::InstalledBlocks` draws the fences of the extensions installed in `ARCHITECT_EXTENSIONS` (`<data dir>/extensions`), embed pages and, with feature `wasi` (`architect-desktop` `node-wasi`), a WASI component's embed view (`ComponentBlocks`; the guest reads its input with `architect_extension_wasi::embed()`), ahead of a distribution's or an embedder's `Blocks`; `Blocks` is a `BlockRenderer` (`claims`, `drawn_by`). Security: a renderer's markup (`Drawn::Html`) is now sanitised (`architect_sites::sanitize`): inline SVG keeps its geometry, classes and `data-*`, and loses scripts, `on…` handlers, `javascript:` URLs and `<foreignObject>`. The share player follows SVG `[data-at]` elements, scrolls the nearest scrolling box by bounding rect and fills the current SVG bar from the theme. Bounds: `sanitize` is linear in its input on any input (at most 256 elements open), an element tree is written to 256 levels, and a component's drawing holds at most 50,000 nodes and 16 MiB.
