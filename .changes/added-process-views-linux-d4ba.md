---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-process-view" = "minor"
"architect-sandbox" = "minor"
"architect-workspace" = "minor"
---

Added process views (Linux, off by default): `architect-process-view` draws a pane from a program of its own, its GPU frames shared as dma-buf textures (shared memory as the fallback), with input, frame callbacks and a stopped view that restarts; `architect-workspace` gains the `external-textures` feature and `paint::view_input`; the lab's "Process views" screenset (feature `process-views`) shows it beside the same scene linked.
