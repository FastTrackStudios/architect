---
"architect-calendar" = "minor"
"architect-dayplan" = "minor"
---

Added `architect-dayplan` (`extensions/dayplan`), the day plans' model: Task's day templates (`DayTemplate`, `TimeBlock` in a `BlockCategory`: three allocatable blocks, meals, reset, sleep), per-date plans (`DayPlan`, `PlannedBlock`) whose blocks are given to a task, a project, a label or a recipe (`BlockAssignment`), ordered rules that pick a day's template (weekday, RRULE, date range, an event's tag) with per-day, per-week and range overrides (`Rules`), where a change reaches only today and after and a past day keeps what it resolved to, and Task's resolver ported as it is (`resolve::reconcile`, `resolve::merge_template`). The Calendar's events read their `tags` (`Event::tags`).
