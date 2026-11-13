---
"architect-app" = "minor"
"architect-web" = "minor"
---

Added the Now Playing bar to the desktop, phone and web apps (#589). The web app plays the active item through its own audio element (its page policy adds `media-src 'self'` only) with Media Session; a view an island claims a note's type with is handed its extension's source on the playhead and its note, and its `PLAYHEAD_*` asks move that source on the deck (`PLAYHEAD_ITEM` and `PLAYHEAD_SET` among them; rate limit unchanged, messages up to 64 KiB), replacing the per-view silent stand-in; a note's `recording` plays on the app's own source; `PLAYHEAD_TOGGLE`, `PLAYHEAD_NEXT` and `PLAYHEAD_PREVIOUS` are commands; a favourite's primary shows Pause while its extension's source plays (the desktop app mirrors it as the extension's runtime state `playing`). Native apps run the bar on a silent clock.
