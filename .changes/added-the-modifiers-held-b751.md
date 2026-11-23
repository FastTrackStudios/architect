---
"architect-extensions" = "patch"
"architect-scene" = "major"
"architect-workspace" = "patch"
---

Added the modifiers held on a region's press and release: `RegionEvent::Modifiers(id, Mods)` (`architect_scene::region::Mods`: `names`, `has("Shift")`, `from_names`, `held`) comes just before a `Press` or `Release` when a key is held, so a Shift-click or ⌘-click on a region reaches the view as one, linked in and in a process; `Press(id)` and `Release(id)` are unchanged. New `Interaction::pressed_with`/`released_with`

Migration: `architect_scene::region::RegionEvent` gained a variant, `Modifiers(id, Mods)`, and is not `#[non_exhaustive]`, so a `match` on it with no `_` arm no longer compiles (error E0004). Add a `RegionEvent::Modifiers(id, mods)` arm (keep `mods` for the `Press` or `Release` that follows) or a `_` arm; `Press(id)` and `Release(id)` are unchanged.
