---
"architect-media" = "minor"
---

Added `architect-media` (#668): core media primitives. `Item` reads a media item from a note's frontmatter (`title`, `subtitle`/`artist`, `duration`, `media`/`recording`, `art`/`cover`; of each pair the first that names a vault file); `Collection` reads a note listing items in named sections (`sets: [{name, songs}]` and `songs` as Studio's setlists write them, or `sections`/`items`), read up to `collection::MAX_ROWS` links and resolved only against the paths a caller passes (`Collection::resolve`, a flat link to a note a section lists left out by its resolved path, `link::Index` for many links, `Resolved::rows`, `collection::queue` for the playhead); `front::fields` gives frontmatter as JSON and `is_listing_fence` names an extension's listing fence.
