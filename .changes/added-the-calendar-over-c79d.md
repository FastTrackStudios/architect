---
"architect-app" = "patch"
"architect-navigator" = "patch"
---

Added the calendar over the real vault: `VaultSource::write` makes periodic notes (`FileOp::CreateFile`, and their folders), and the lab sets the demo vault's periodic notes (`Journal`, `[Daily]/YYYY-MM-DD`, `[Weekly]/GGGG-[W]WW`, …) while they are still the defaults; five demo daily notes have a feature image.
