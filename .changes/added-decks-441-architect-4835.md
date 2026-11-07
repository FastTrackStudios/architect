---
"architect-extensions" = "minor"
"architect-workspace" = "minor"
---

Added decks (#441): `architect_workspace::Deck`, `Decks`, `DeckState` and `DeckContext`, one loaded context each, shown in a screenset with a per-context override (`DeckState::record`/`show`, `open` with `Load::Here`/`Load::NewDeck`, `set_screenset`, `save_as_screenset`, `reset`); `store::ContextStore`, `load_decks`, `save_session`; `home::ContextFiles` (`.architect/deck.json`) and `home::FileStore`; the `DECK_*` actions (`control::deck`, `HeadlessDecks`); and the `[[deck_templates]]` manifest point (`DeckTemplateDecl`, `Contributions::deck_templates`)
