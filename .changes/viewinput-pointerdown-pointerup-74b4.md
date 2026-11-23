---
"architect-extensions" = "minor"
"architect-process-view" = "minor"
"architect-workspace" = "major"
---

ViewInput::PointerDown, PointerUp and PointerMove carry the modifier keys held (mods, named Mod, Shift, Alt, Ctrl), filled on the desktop, the web and across a process view. A match on these variants names the field or ends with ..

Migration: `architect_workspace::ViewInput`'s `PointerDown`, `PointerUp` and `PointerMove` gained a field, `mods: Vec<&'static str>`, so a pattern that lists their fields without `..` no longer compiles (error E0027, "pattern does not mention field `mods`"). Add `mods` to the pattern where the view reads the modifiers (a Shift-click, a drag that adds), or end it with `..` (`ViewInput::PointerDown { x, y, button, .. }`). Code that builds these variants (a test, a host forwarding input) sets `mods: vec![]` or the held names. `ViewInput::Wheel` and `ViewInput::Key` are unchanged.
