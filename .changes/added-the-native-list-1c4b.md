---
"architect-desktop" = "minor"
"architect-media" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
"architect-web" = "minor"
---

Added the native list of a collection (#668): `architect_media::ui::MediaList` (feature `ui`) draws a collection note at its head in the web app, unless an island claims its type, and plays the whole list on the playhead, with the `sets`/`songs` property rows hidden and a `studio-setlist`-style listing fence folded out of the reading. A share link's guest gets the same list: `ListedNote` gains `subtitle`, `duration_ms`, `media` and `sets` (`ListedSet`), resolved by the node only among the link's notes; a note's `media`/`recording` file is one of its files on a link, and `cover` is its art when it has no `art` (`architect_sites::art::art_link`, `art` before `cover` as `architect_media::Item` reads them). Every listing fence is cut when a site or a share link is built (`architect_media::fence::within`, found with `pulldown_cmark` as the page is): a row naming a note outside the site is gone, and in a row kept only the readable note, a served file or plain text stays. The page (on a share's own domain and a published Site too), its plain blocks, its text and an extension's drawn block all read the cut rows, and the node cuts what it hands a guest (`ShareGuest::read`) to the link's notes again.

Behaviour change for existing links: a recording or cover a note names only by `media:`, `recording:` or `cover:` (not linked in its own body) is now served on a link, but at the Reference layer when no `[[share_layers]]` rule places it (`listing::Entry::listening`), whatever another note's body links, so a link set to Page serves none it did not serve before, and a Reference or Full link (the default is Reference) now serves them.
