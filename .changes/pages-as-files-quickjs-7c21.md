---
"architect-pages" = "minor"
"architect-extensions" = "minor"
"architect-scripting" = "minor"
"architect-sandbox-board" = "minor"
"architect-app" = "minor"
---

Pages as editable files (#273): `architect-pages` — the page API as `#[architect::rpc]` services (PageHost, PageView) over facet types, `architect.d.ts` and `architect.d.luau` generated from their vox descriptors (a drift test), TSX/JS pages on QuickJS (one runtime per page on its own thread, ES imports inside the page's folder, no Node APIs, a per-call interrupt, workers, console.log to the page's log) with TS/JSX transpiled by oxc and source-located errors, Luau element pages, reload on save keeping the last good page and writing `<file>.error`, overrides under `<app dir>/pages/`, read-only built-ins with clone-to-edit, and `PAGES_STATUS` / `PAGES_CHECK` / `PAGES_CLONE`. Extensions: `[[views]] page`, `interface::ViewPages` (additive), and a remote view's frame now asks the window to redraw. The lab's Board tab is `apps/lab/board/pages/board.tsx`.
