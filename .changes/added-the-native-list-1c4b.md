---
"architect-desktop" = "minor"
"architect-media" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
"architect-web" = "minor"
---

Added the native list of a collection (#668): `architect_media::ui::MediaList` (feature `ui`) draws a collection note at its head in the web app, unless an island claims its type, and plays the whole list on the playhead, with the `sets`/`songs` property rows hidden and a `studio-setlist`-style listing fence folded out of the reading. A share link's guest gets the same list: `ListedNote` gains `subtitle`, `duration_ms`, `media` and `sets` (`ListedSet`), resolved by the node only among the link's notes; a note's `media`/`recording` file is one of its files on a link, and `cover` is its art when it has no `art` (`architect_sites::art::art_link`, `art` before `cover` as `architect_media::Item` reads them). What the node hands a guest of a note (`ShareGuest::read`: its blocks and the text a view reads) has every listing fence cut to the link (`architect_media::fence`): a row naming a note outside the link is gone, and a file the link does not serve is emptied from a row kept.

Behaviour change for existing links: a recording or cover a note names only by `media:`, `recording:` or `cover:` (linked in no note's body) is now served on a link, but at the Reference layer when no `[[share_layers]]` rule places it (`listing::Entry::listening`), so a link set to Page serves none it did not serve before, and a Reference or Full link (the default is Reference) now serves them.
