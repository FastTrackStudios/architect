---
"architect-sites" = "patch"
---

Improved a share page's chart cursor: a follow item with `data-glide` (and optional `data-until`) now moves smoothly across the beat at the playhead, once per frame, by a single transform (Studio's chart playhead line, as Session draws it). A follow box nested in another one now follows only its own items. A media element first played by hand is now followed every frame.
