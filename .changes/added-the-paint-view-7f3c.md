---
"architect-workspace" = "minor"
"architect-app" = "minor"
---

Added the `paint` view kind: an anyrender scene each frame, drawn in-process by Blitz in a pane with a real `ViewCtx` (geometry, input and IME, frame clock, theme tokens), one display list to paint and hit-test from, p50/p99 frame times, and typed host data (`HostData<T>`, `PaneHandle::state_as`); the lab's Embedded views draw 10 000 rectangles with it; `paint-web` adds a host-driven WebGL2 canvas and `paint-web-ui` a Dioxus browser pane with input, live theme and coalesced wake support.
