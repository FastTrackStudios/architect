---
"architect-extensions" = "minor"
"architect-workspace" = "minor"
---

A paint view runs unchanged linked in or as a process: `linked::PaintPane` and `linked::use_pane_state` in an extension's own view; a process view is told its pane's id and state; the process side gets the same `ViewCtx` (theme, extras, `PaintWake`) and every input kind with its modifiers. A paint view's wheel is now signed as the web's and placed in the view, linked in too (it was winit's sign at the window's point).
