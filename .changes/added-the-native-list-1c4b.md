---
"architect-desktop" = "minor"
"architect-media" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
"architect-web" = "minor"
---

Added the native list of a collection (#668): `architect_media::ui::MediaList` (feature `ui`) draws a collection note at its head in the web app, unless an island claims its type, and plays the whole list on the playhead, with the `sets`/`songs` property rows hidden and a `studio-setlist`-style listing fence folded out of the reading. A share link's guest gets the same list: `ListedNote` gains `subtitle`, `duration_ms`, `media` and `sets` (`ListedSet`), resolved by the node only among the link's notes; a note's `media`/`recording` file is one of its files on a link, and `cover` is its art when it has no `art` (`architect_sites::art::art_link`).
