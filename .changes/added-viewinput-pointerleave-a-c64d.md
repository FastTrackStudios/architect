---
"architect-extensions" = "minor"
"architect-process-view" = "minor"
"architect-sandbox-sketch" = "minor"
"architect-ui" = "minor"
"architect-workspace" = "minor"
---

Added `ViewInput::PointerLeave`: a paint view hears the pointer leave its pane (after a move to where it went), linked in, in a process copy (`RemotePaint`), in a browser's canvas and in a `kind = "process"` program, where it crosses as a move to NaN that older programs read as a move; new `architect_workspace::paint::left_view`
