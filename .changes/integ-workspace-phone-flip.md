---
"architect-workspace" = "patch"
---

Resizing a window across a phone's width with a workspace on show no longer panics (rules of hooks): `WorkspaceView` asked for the shell only once it was a phone.
