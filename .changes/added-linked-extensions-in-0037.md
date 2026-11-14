---
"architect-calendar" = "minor"
"architect-command-palette" = "minor"
"architect-navigator-api" = "minor"
"architect-dayplan" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-files-extension" = "minor"
"architect-notify" = "minor"
"architect-projects" = "minor"
"architect-tasks" = "minor"
"architect-theme-picker" = "minor"
"architect-threads" = "minor"
"architect-time-tracker" = "minor"
"architect-web" = "minor"
"xtask" = "minor"
---

Added linked extensions in the browser (#495): `architect_web::run(Distribution)` runs a distribution's linked extensions in the web app on the same host as the desktop (their actions in ⌘K, views as panes, settings sections), feature `start` for the Architect app's own start; `architect-extensions` feature `linked-core` (the trait without a host) and `linked::MaybeSend`, its `host` building for `wasm32-unknown-unknown` without Luau; `cargo xtask dist web` builds a distribution crate's own wasm when it has a `web` feature. Natively nothing changes; in a browser `linked::HostFuture` and `linked::ActionFuture` are not `Send`. The Command Palette builds for the browser too (`linked-core`, `guest` only off it). In a browser the host's clock is the app's timer (`Host::tick`), its consent starts empty (`Grants::in_memory`), and its time is `web-time`'s.
