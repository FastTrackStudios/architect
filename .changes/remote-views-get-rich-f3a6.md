---
"architect-extension-wasi" = "minor"
"architect-extensions" = "minor"
"architect-pages" = "minor"
"architect-sandbox-kanban" = "minor"
---

Remote views get rich input (#176): pointer, touch, wheel, scroll and input-method composition events reach a process, WASI or page view and the browser applier (wire 7; WASI components rebuild against the new world). A press holds the pointer on the element pressed, fast moves are coalesced while the view is busy, and a field in a view gets the window IME and clipboard. Kanban-lite, a drag-and-drop example extension.
