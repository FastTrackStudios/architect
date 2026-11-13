---
"architect-playhead" = "minor"
---

Added architect-playhead and the built-in extension `playhead` (#589): one playhead per app where any extension puts its queue as its source (`PLAYHEAD_SET`, `PLAYHEAD_PLAY`, `PLAYHEAD_PAUSE`, `PLAYHEAD_TOGGLE`, `PLAYHEAD_SEEK`, `PLAYHEAD_NEXT`, `PLAYHEAD_PREVIOUS`, `PLAYHEAD_ITEM`, `PLAYHEAD_STATE`, `PLAYHEAD_SOURCES`, `PLAYHEAD_ACTIVATE`; service `architect.playhead`; app event `playhead.changed`), each extension moving only its own; the `Clock` seam (`Silent`, `Follower`), and feature `ui` with the Now Playing bar (`ui::NowPlayingBar`, `ui::use_playhead`).
