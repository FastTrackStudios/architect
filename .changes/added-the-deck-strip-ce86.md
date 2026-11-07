---
"architect-workspace" = "minor"
---

Added the deck strip (#441): `use_decks`, `DeckStrip`, `DeckHandle` and `use_deck_context` (feature `ui`): a tab per deck over the main window (the title row there), drag to reorder, ⌘1–9 and ⌃Tab to switch, duplicate, pop a deck out into a window, switch its screenset, save as screenset, reset; `WorkspaceView` prop `under_strip`; `DecksConfig::actions` puts the `DECK_*` actions on the app's Action List, run by the strip. Once two decks are open the strip shows, and screensets load with ⌘⌥1–9 and a group's tabs cycle with ⌃⌥Tab; a phone's switcher lists the decks
