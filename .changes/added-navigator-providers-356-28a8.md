---
"architect-app" = "minor"
"architect-extensions" = "minor"
"architect-navigator" = "minor"
"architect-navigator-api" = "minor"
"architect-projects" = "minor"
"architect-tasks" = "minor"
---

Added navigator providers (#356): pages and extensions fill the navigator, the app's one left sidebar, with their own containers and items. The MIT `architect-navigator-api` crate holds `NavigatorProvider` (`snapshot`, `open`, `move_items`, `create`), `Snapshot`, `Node`, `Item`, `Open` and `ActionProvider`, so a provider never links the GPL navigator; extensions declare `[[navigator_providers]]` (`NavigatorProviderDecl`) and serve one with `linked::Builder::navigator`; `architect-navigator` draws any provider with `ProviderSource`. The app's sidebar switches to a page's provider and keeps each one's selection; the Projects extension provides projects then their tasks (`PROJECTS_NAV_*`), the Tasks extension status groups then their tasks (`TASKS_NAV_*`).
