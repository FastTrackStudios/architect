---
"architect-pages" = "minor"
---

Added a DOM in a page's runtime, `import { document } from "architect/dom"` (`modules::ARCHITECT_DOM`), so Preact or Solid render in a page natively (QuickJS) and in a browser's Web Worker: what the framework does to it is drawn as the page's element tree and edits, and pane events come back to its listeners as DOM events. A page's default export may now be `document.body` as well as a component. Every page now has `requestAnimationFrame`/`cancelAnimationFrame` (a 16 ms timer of the page's own, replacing a Web Worker's) and `queueMicrotask`; `architect.d.ts` declares them and the `architect/dom` module.
