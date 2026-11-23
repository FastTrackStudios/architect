---
"architect-extensions" = "patch"
"architect-process-view" = "patch"
"architect-workspace" = "patch"
---

Fixed process views (#838): a view's program no longer inherits another's socket (the guest end stays close-on-exec, `sock::hand_to` clears it in the child alone), so a program's death is always seen; a pop-out window draws process views (`ProcessViews` carried across the window bridge); a session ended for a reason (`host::retire`: a grant taken back, its extension unloaded) says why and its Restart starts nothing; the pane checks again while open and offers Allow when only the person's yes is missing (`process_view::needs_grant`, `grant_program`); the program gets the app's theme (`paint::pane_tokens`, now public) and the demo follows it; the frames test takes turns on the GPU with a watchdog that names the step it is stuck at.
