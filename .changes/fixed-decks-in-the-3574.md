---
"architect-app" = "patch"
---

Fixed decks in the app (#468): `DECK_OPEN {context: {kind: project, id}}` opens the project's page in the Project screenset, as a middle-click does, and a project's row in the navigator's Projects loads onto a deck on a middle-click or ⌘-click; the navigator follows the deck on show (a project's row in Projects, a song's setlist, the whole navigator on the vault's deck), each deck keeping its own selection; the Setlists demo dates songs by their own dates or their file's mtime.
