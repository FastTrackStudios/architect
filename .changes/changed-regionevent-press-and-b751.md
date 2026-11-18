---
"architect-extensions" = "patch"
"architect-sandbox-sketch" = "patch"
"architect-scene" = "major"
"architect-workspace" = "minor"
---

Changed `RegionEvent::Press` and `RegionEvent::Release` to carry the modifiers held: `Press(id, Mods)`, `Release(id, Mods)` (`architect_scene::region::Mods`: `names`, `has("Shift")`, `from_names`, `held`), so a Shift-click or ⌘-click on a region reaches the view as one, linked in and in a process; new `Interaction::pressed_with`/`released_with`. Migration: match `Press(id, _)`
