---
"architect-app" = "minor"
"architect-bases" = "none"
"architect-cookbook" = "none"
"architect-distribution" = "minor"
"architect-extensions" = "none"
"architect-mealplan" = "none"
"architect-pages" = "none"
"architect-web" = "minor"
"xtask" = "none"
---

Changed: the demo vault is the extension showcase's vault (its Showcase note, the chapter notes and .architect/extensions.toml now live in apps/ui/demo-vault); the separate showcase vault and the first run's "Explore the extension showcase" are gone, and architect_app::vaults::showcase and architect_web::Route::Showcase are deprecated (the demo vault, Route::Demo). Added: devenv up loads every showcase extension as a development copy (scripts/showcase.sh dev-copies) and builds the app with WASI, so the WASI counter runs.
