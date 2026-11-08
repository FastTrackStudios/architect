---
"architect-extensions" = "minor"
"architect-workspace" = "minor"
---

Added tab kinds and scope tabs (#476): `TabKind`, `TabKinds`, `TabFace`, `TabWidth` and `[[tab_kinds]]` (`TabKindDecl`); `Decks::general` holds the general tabs (no scope tab active), with `DeckState::show_general`, `set_general` and `Decks::fold_general`; `DeckContext::properties`. Changed the deck actions' prefix from `DECK_` to `SCOPE_` (`scope_word!`), added `SCOPE_GENERAL` and `TAB_KINDS`; `SCOPE_NEXT`/`SCOPE_PREV` go round the general tabs too.
