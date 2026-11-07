---
"architect-calendar" = "minor"
"architect-dayplan" = "minor"
---

Added day plans to the Calendar's day and week views (#444): each day's blocks drawn under its events in their categories' colours with the time tracked against the time planned (`architect_calendar::plan_ui::PlanLayer`), the day's template and why on its head with a menu to choose another, go back to the rules or reset to the template (`TemplateBadge`), and a block's panel to give it to a project, a task, a recipe or a label (`PlanPanel`). Day Plans gets its views, the template editor (`templates`) and the rules (`rules`, with the next two weeks' templates and why), opened by `DAYPLAN_OPEN`. Fixed: the Calendar's day, week, month and agenda views draw again when what it read changes (they take its revision). `architect_dayplan::refresh` moves the revision only on a change.
