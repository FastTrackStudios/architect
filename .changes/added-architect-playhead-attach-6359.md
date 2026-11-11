---
"architect-sites" = "minor"
---

Added `architect.playhead.attach(source)` to the Sites player (`features/sites/assets/player.js`, #528): an outside clock (an audio engine on the page) drives the playhead in place of the page's media (`time`, `playing`, `rate`, `duration`; `seek`, `play`, `pause` asked of it; `src` the media it stands in for); the chart and lyrics follow it, the page's player yields (media paused, bar hidden); the handle's `from` carries the page's position, `changed()` repaints, `detach()` gives the clock back where the engine was; snapshots say `source`.
