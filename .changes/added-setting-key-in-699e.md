---
"architect-extensions" = "minor"
"architect-process-view" = "minor"
---

Added `{{setting:<key>}}` in a process view's `args` (#838): one of the extension's own settings is put in when the program starts, and a change starts it again (`extension::refresh`); `Host::setting_value`, `manifest::setting_refs` and `with_settings`. Changed when a process view's program stops: when its pane leaves the workspace (`extension::register_pane`, `fate`; a move to another group or a pop-out window never restarts it), with a 2 s `BACKSTOP` where there is no workspace to ask, in place of a 10 s timer; the reaper no longer touches the lab's own sessions. Added `host::keys` and `host::away`.
