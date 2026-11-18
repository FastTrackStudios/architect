---
"architect-app" = "minor"
"architect-extensions" = "minor"
"architect-process-view" = "minor"
---

Added process views on macOS (#838): frames as IOSurfaces sent as Mach ports and drawn without a copy, shared memory beside them where a surface cannot be imported (architect_process_view::surface, wire::Transport::IoSurface); the platform gate (process_view::not_here) now lets Linux and macOS through. Windows still says why.
