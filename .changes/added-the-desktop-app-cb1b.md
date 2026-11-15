---
"architect-desktop" = "minor"
---

Added the node to the desktop app's own process: the window starts `node::start_parts`, the one implementation `serve` uses, from the same configuration over defaults for a person's machine (`node::config::app_defaults`: HTTP on `127.0.0.1:7420`, state in `<app dir>/node`, the machine's platform), on the account as the app's signed-in account (`node::AccountSource`) unless a session file is configured. New: `node::Parts`, `node::AccountSource`, `node::start_parts`, `Config::load_with`, `Config::platform` (`ARCHITECT_PLATFORM`). Not in the sandbox; not on iOS yet (#820).
