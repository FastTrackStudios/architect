---
"architect-extensions" = "minor"
"architect-process-view" = "minor"
"architect-ui" = "minor"
"architect-workspace" = "major"
---

Added `ViewInput::PointerLeave`: a paint view hears the pointer leave its pane (after a move to where it went), linked in, in a process copy (`RemotePaint`), in a browser's canvas and in a `kind = "process"` program, where it crosses as a move to NaN that older programs read as a move; new `architect_workspace::paint::left_view`

Migration: `architect_workspace::ViewInput` gained a variant, `PointerLeave`, and is not `#[non_exhaustive]`, so a `match` on it with no `_` arm no longer compiles (error E0004). Add a `ViewInput::PointerLeave` arm (a hover that should clear when the pointer goes) or a `_` arm. A host that builds `ViewInput` for a view sends `PointerLeave` when the pointer leaves the pane.
