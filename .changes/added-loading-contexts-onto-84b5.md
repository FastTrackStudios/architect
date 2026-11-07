---
"architect-app" = "minor"
"architect-navigator" = "minor"
"architect-navigator-api" = "minor"
---

Added loading contexts onto decks from the navigator (#441): `OpenIn::NewDeck` (a ⌘-click on a deck row, a middle-click on any row; `DeckRows` says which rows load as decks), `architect_navigator_api::Open::Context` for a provider's items, and in the app the vault, projects and (sandbox) a setlist's songs as decks with the session per device and each layout kept with its context
