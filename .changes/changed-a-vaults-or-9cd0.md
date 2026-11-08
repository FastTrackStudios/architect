---
"architect-navigator" = "patch"
---

Changed a Vaults/ or Assets/ row, and a note from another org in a list, to show its org by a light wash of the org's colour (`OrgBadge::tint`) instead of the org's initial badge; the org is the row's `data-org` and its description. A theme sets how strong the wash is with the number `navigator.org-wash` (0 to 1, 0.08 by default, 0 turns it off; `NUMBERS`).
