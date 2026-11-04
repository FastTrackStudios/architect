---
"architect-canvas" = "patch"
---

A script canvas (`use_script_canvas`) draws its first frame only when something asks for one: its view's spaces, a pointer, a window event or the script's own wake. A theme change alone no longer runs a frame before that, so a script whose view is hidden (the lab's marker lane before the Arrangement shows) no longer opens its tool window at start. After its first frame, a theme change still redraws it, and a theme that did not change causes no frame.
