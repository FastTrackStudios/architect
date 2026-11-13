---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-favourites" = "minor"
"architect-navigator" = "minor"
"architect-shell" = "minor"
"architect-web" = "minor"
---

Security: a note type's favourites name only what their own extension may reach, as every other contribution that names an action: its own views, and an action in its namespace or one its capabilities name up to their effect, never a destructive one (`architect_favourites::Declarer`, `Declarer::may_target`, `Declarer::may_open_view_of`, `Manifest::declarer`); anything else is left out, natively and in a browser. `GET /_ext/favourites.json` now lists each declaration as `Listed { by, favourites }` and `start_with_shell` and `Distribution::favourites` take and make `Listed`. Added `architect_shell::Shows`, `Favourites::current` (the declared tab whose place is on show, now `aria-current` in the native app, and on a tablet's rail and sidebar rows and a desk's ribbon too), `PlaceOpened` and `set_note` (a `{place}` tab keeps the set of the note it was opened from), `SourceEvent::touches` (editing a note's `type` updates the slots), `Island::views` (the browser opens any view an island lists, handed the note and its playhead source as a claimed view is; the song views and the `tags: [song]` check are gone), a 64-character limit on `note_type` (`MAX_TYPE`) and a 256 KiB limit on the listing the browser reads. Fixed the native primary's second face, which never showed (an extension's runtime state is `ext:<id>`'s).
