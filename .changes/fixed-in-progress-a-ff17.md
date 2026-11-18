---
"architect-extensions" = "patch"
"architect-workspace" = "patch"
---

Fixed (in progress) a process view in the active pane not hearing keys until clicked (#889): the view takes the window focus when its pane is active (PaneHandle::is_active, RemoteDocument::take_focus).
