---
"architect-extensions" = "minor"
---

Added toolbar images that follow the theme: an SVG `image`'s `currentColor` is the item's ink, and `tint = true` (`ToolItem::tint`) draws every colour it names in that ink (`Toolbars::tinted`), so a glyph drawn for dark chrome stays visible on light. The sketch sandbox has a timeline toolbar with one, and `LAB_SKETCH_NUDGE` bound to N in the timeline's view context
