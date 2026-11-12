---
"architect-desktop" = "patch"
"architect-sites" = "patch"
---

Fixed Loop seeking itself to death on a section shorter than 0.25 s (sections Loop holds are long enough and in order; a re-entrancy guard), beats closer than 50 ms scheduling a flood of clicks, Loop staying enabled while an outside clock holds the playhead, playlists joining when they do not opt in (data-playlist-join, or a legacy studio-setlist), a song listed twice playing its first copy, art not reaching a page with no recordings, and a bare art: name preferring a vault-root file over the note's own. Added a row's menu (data-track-more: Open chart, Lyrics, Song page and downloads; a sheet on a phone), the key on a row's art tile (data-key, data-track-key, data-track-transposed), 44 px icon targets, colour fallbacks for Safari 16.0/16.1, the now-playing view's fallback bar without times and its previous/play/next as one joined control
