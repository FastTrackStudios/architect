---
"architect-app" = "minor"
"architect-media" = "minor"
---

Added the native list of a collection in the desktop and phone apps (#668): a collection note draws as `architect_media::ui::MediaList` as the fallback of its note type's `EmbedView`, its covers read from the vault as `data:` URLs (also the Now Playing card's art), its `sets`/`songs` rows hidden from the Properties bar and its listing fence folded in the editor (`architect_media::listing_fences`); the demo vault on disk gets the demo's silent recordings, so its setlist plays.
