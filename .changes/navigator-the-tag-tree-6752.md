---
"architect-navigator" = "minor"
"architect-navigator-extension" = "minor"
---

Navigator: the tag tree and the property browser (#337). Tags nested by /, a Tags and an Untagged row, A to Z or by frequency, a hand-made root order, scoped to the selection; add, remove and clear tags on notes, rename and delete a tag, a new note in a tag, notes and tags dragged onto tags; the property keys named in the settings with their values and counts, a property set on notes; Cmd/Ctrl-click into the search with AND, Shift with OR (Query). Breaking: architect_navigator::grouping is gone (list::property_groups over vault_live::grouping::group_by), the model depends on vault-live and is native only, and FileOp, NavTarget, NavRowKind, ListContext and ViewState gain variants and fields.
