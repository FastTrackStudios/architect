---
"architect-desktop" = "minor"
"architect-sites" = "minor"
"architect-web" = "minor"
---

A note's extension view by its type in the web app: an island claims note types for a view (SiteIsland::embed, listed as embeds in /_ext/islands.json), and the app draws that view at the head of a note whose type it claims, given the note and its source's playhead state; frames may also ask PLAYHEAD_SET and PLAYHEAD_ITEM (architect.playhead v0.1, #589), held by a silent per-view stand-in until the deck lands. island_node takes ARCHITECT_ISLAND_EMBEDS.
