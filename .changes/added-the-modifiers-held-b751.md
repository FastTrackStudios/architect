---
"architect-extensions" = "patch"
"architect-sandbox-sketch" = "patch"
"architect-scene" = "minor"
"architect-workspace" = "patch"
---

Added the modifiers held on a region's press and release: `RegionEvent::Modifiers(id, Mods)` (`architect_scene::region::Mods`: `names`, `has("Shift")`, `from_names`, `held`) comes just before a `Press` or `Release` when a key is held, so a Shift-click or ⌘-click on a region reaches the view as one, linked in and in a process; `Press(id)` and `Release(id)` are unchanged. New `Interaction::pressed_with`/`released_with`
