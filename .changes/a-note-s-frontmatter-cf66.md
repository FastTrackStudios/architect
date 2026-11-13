---
"architect-navigator" = "patch"
---

A note's frontmatter `color` (and a project note's) is drawn only when it is a CSS colour (`state::looks::css_colour`: hex, `rgb()`/`hsl()` of numbers, a named colour), never a style declaration. `markdown::split_block`: a note's frontmatter block (always ending with a line end) and its body.
