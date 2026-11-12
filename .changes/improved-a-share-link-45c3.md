---
"architect-desktop" = "minor"
"architect-sites" = "minor"
---

Improved a share link's playlist to read as a streaming app's: playlists with only headings between them are one list (each heading a data-playlist-section label row), rows show number, art tile, title over artist, length and a quiet link to the song's page, and a song with no recording (data-page without data-track) stays in the list and opens its page. Added a note's art: its art: property names an image beside it, one of its files under the scope and layer rules (architect_sites::art, listing::Entry::art, Meta::art); the node gives each listed page's art in the queue (art) and the player lays it over the row's tile. Changed the player's bar on a playlist's page to a floating card (art, song, Click, Loop, play; previous/next in the now-playing view), added Click (a Web Audio metronome on the view's [data-at][data-beat] marks) and Loop (the section playing, from data-sections), made the now-playing view's chart fill the width with one compact row of controls and lyrics/page icons in its head, removed data-playlist-play, and tightened a page's title block on a phone
