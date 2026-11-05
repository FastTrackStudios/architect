---
"architect-extensions" = "minor"
"architect-pages" = "minor"
---

Added TSX/JS pages in a browser: feature `web` of `architect-pages` runs a page in a Web Worker on the browser's own engine with the same prelude and `architect.d.ts` (`web::WebPages`, `WebPage`, `PageFolder`, `Limits`), its host calls checked by `architect_extensions::web::WebHost` (new `WebHost::may_run`, `WebHost::list`) and drawn by `architect_extensions::web::Pane` (now public, its events typed `wire::ViewEvent`; a field's `value` and `checked` are set as properties too); a runaway unit of work terminates the worker and starts the page again. Also feature `tsx` (oxc alone, wasm-clean; `js` now enables it), `modules::prepare` (a page's folder as linkable modules), and `frame` (the banner and event mapping every runner shares).
