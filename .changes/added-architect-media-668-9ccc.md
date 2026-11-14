---
"architect-media" = "minor"
---

Added `architect-media` (#668): core media primitives. `Item` reads a media item from a note's frontmatter (`title`, `subtitle`/`artist`, `duration`, `media`/`recording`, `cover`/`art`); `Collection` reads a note listing items in named sections (`sets: [{name, songs}]` and `songs` as Studio's setlists write them, or `sections`/`items`), resolved only against the paths a caller passes (`Collection::resolve`, `Resolved::rows`, `collection::queue` for the playhead); `front::fields` gives frontmatter as JSON and `is_listing_fence` names an extension's listing fence.
