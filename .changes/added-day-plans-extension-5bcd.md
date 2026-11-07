---
"architect-app" = "minor"
"architect-calendar" = "minor"
"architect-dayplan" = "minor"
---

Added Day Plans as a first-party extension (`architect_dayplan::DayPlans`, id `dayplan`, service `architect.dayplan`), linked into the app: `DAYPLAN_GET` and `DAYPLAN_UPSERT` (Task's `get_day_plan` and `upsert_day_plan`), `DAYPLAN_DAYS`, `DAYPLAN_ASSIGN`, `DAYPLAN_SET_BLOCK`, `DAYPLAN_RESET`, `DAYPLAN_TEMPLATES`, `DAYPLAN_TEMPLATE`, `DAYPLAN_TEMPLATE_RULES`, `DAYPLAN_SET_WEEKDAY`, `DAYPLAN_OVERRIDE` and `DAYPLAN_RESOLVE`, over notes in `Calendar/Day templates/`, `Calendar/Day rules.md` and `Calendar/Day plans/`. `CALENDAR_AGENDA` gives each event's `tags`. The demo vault has day templates, rules, an override, an edited day, recipes and events.
