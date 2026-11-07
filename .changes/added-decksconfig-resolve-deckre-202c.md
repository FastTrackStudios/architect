---
"architect-navigator" = "minor"
"architect-workspace" = "minor"
---

Added `DecksConfig::resolve` (`DeckResolve`, also `control::deck::Kept::resolve`, so `DECK_OPEN` fills in a context given by type and id) and `DecksConfig::on_fresh` (told of every context loaded with nothing kept); `NavigatorState::show_folder`, `NavigatorState::set_active_with` and `Navigator::set_active_with`; a date of 0 groups under "No date", not 1969. Pre-1.0 API change: a `DecksConfig { .. }` literal needs the two new fields (`None` for both keeps the old behaviour).
