---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-mobile" = "minor"
"architect-sandbox" = "minor"
"architect-sandbox-media" = "minor"
---

Added `architect-distribution`: a distribution of Architect as code (`Distribution`: name, app name, folder, bundle id, icon, and the extensions and `Bundle`s compiled in; `architect()` is the Architect app with its first-party extensions). `architect_desktop::run` takes a `Distribution` in place of `architect_app::Mode` (removed, with `architect_app::mode::migrate_lab_folder`: use `Distribution::adopts` / `adopt_folder`); `apps/ui`'s `use_lab_extensions` links the distribution's list; `<bin> distribution` prints what it is. The registries a build installs from are the distribution's too (`Distribution::registry`, `without_registries`, `without_installs`; every distribution has the store's, `architect_distribution::store`, which moved here from `apps/ui` with `STORE_REGISTRY` / `STORE_DIR`; `app_registries` takes the distribution). The node's share links and Sites draw the fences its compiled-in extensions' embed pages claim, and the distribution's own renderers (`node::start_with`, `Distribution::blocks`, #488). Fixed `architect-extensions`' `guest` without the workspace-hack: it now turns on dioxus-html's `serialize` itself.
