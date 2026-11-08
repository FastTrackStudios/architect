---
"architect-navigator" = "minor"
"architect-navigator-api" = "minor"
"architect-workspace" = "minor"
---

Added `architect_navigator_api::PROJECT_ICON` (Lucide `box`): one project glyph that replaces the folder icon on every project row, a vault folder with a `project.md` and that note included, with the project's own `icon` and `color` over it (`NavigatorState::project_page`, `PROJECT_NOTE`); `FolderMeta::colour` from a provider's `Node::colour`; Recent Files rows draw their note's icon; `DeckContext::colour`, and a deck's tab draws its context's icon.
