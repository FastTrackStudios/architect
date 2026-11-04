---
"architect-canvas" = "minor"
"architect-extensions" = "minor"
"architect-scripting" = "minor"
"architect-shell" = "minor"
"architect-workspace" = "minor"
---

Responsive by design: no Luau runs on its caller's thread (a pool for runs, a thread per live script), workers ([[workers]], worker.spawn, Guest::spawn_work / use_work) for heavy work with progress and cancellation, a per-frame budget (Limits::frame), and every extension view measured (ViewMeter, Meters, EXT_VIEWS) and marked busy (BusyBadge) past its budget. Breaking: Limits gains frame and time; wire 5 answers every event with a frame.
