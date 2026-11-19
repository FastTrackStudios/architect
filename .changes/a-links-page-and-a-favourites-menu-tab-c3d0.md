---
"architect-favourites" = "minor"
"architect-distribution" = "minor"
"architect-sites" = "patch"
"architect-shell" = "none"
"architect-web" = "minor"
"architect-app" = "patch"
---

Added a menu as a favourites tab's target: `target = { menu = [entries] }` (`architect_favourites::Target::Menu`, at most `MAX_MENU` entries, each a view or an action, never another menu), drawn above its tab on a link's guest page (a phone's More); the desktop shell leaves such a tab out until it can draw it. A share link's page is the note: one thin title line, the link's notes and files in the sidebar, the root note's type's favourites as the bottom tabs, each a view of the declaring extension's island. `Distribution::favourites_of(manifest)` lists the favourites of a manifest that is not a compiled-in extension's (its island's).

Migration: `architect_favourites::Target` has a new variant, `Menu`, so an exhaustive `match` on it adds an arm.
