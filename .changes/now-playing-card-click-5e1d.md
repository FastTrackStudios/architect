---
"architect-playhead" = "minor"
"architect-shell" = "patch"
"architect-web" = "minor"
---

The Now Playing bar is a share page's player card: the song's art, title over subtitle, the source's two slots and play, its progress a thin line along its bottom edge; previous, next, the position and the source switcher are its detail, opened by `^` (or a press on the song). A source asks for its slots with `PLAYHEAD_SET`'s `slots` (at most two of `click` and `open`; none draws `open`), and an item may give its `tempo` (`{bpm, at, meter}`); the web app sounds Click on the active item's beats while it plays. A `PLAYHEAD_SET` without `index` keeps the current item when it is still in the queue (a view mounted again no longer sends the source back to its first item). Fixed: the favourites' round button sat under the bar at desk width; a pane going from one island view to another kept the last view's frame (keyed by island and view); an island view's frame was drawn on an opaque box (the frame's colour scheme is now the page's).
