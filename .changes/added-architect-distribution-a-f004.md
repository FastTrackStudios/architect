---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-mobile" = "minor"
"architect-sandbox" = "minor"
"architect-sandbox-media" = "minor"
---

Added `architect-distribution`: a distribution of Architect as code (`Distribution`: name, app name, folder, bundle id, icon, and the extensions and `Bundle`s compiled in; `architect()` is the Architect app with its first-party extensions). `architect_desktop::run` takes a `Distribution` in place of `architect_app::Mode` (removed, with `architect_app::mode::migrate_lab_folder`: use `Distribution::adopts` / `adopt_folder`); `apps/ui`'s `use_lab_extensions` links the distribution's list; `<bin> distribution` prints what it is. The node's share links and Sites draw the fences its compiled-in extensions' embed pages claim, and the distribution's own renderers (`node::start_with`, `Distribution::blocks`, #488). Fixed `architect-extensions`' `guest` without the workspace-hack: it now turns on dioxus-html's `serialize` itself.
