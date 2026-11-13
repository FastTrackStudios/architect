---
"architect-web" = "minor"
---

Fixed: on the org's vaults typing is merged by Loro at the version the text area showed (`live_text::LiveText`, `merge_at`; `rebase` and `apply` removed), so remote edits in several places, a delete and an IME composition keep everyone's text; a new note never empties a note of that name made since the listing (`Notes::create`, `create_first`); Sign out revokes the sign-in at the issuer; a note opened first no longer takes the navigator's place; New note from the palette or its shortcut no longer stops the app.
