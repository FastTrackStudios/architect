---
"architect-app" = "minor"
"architect-calendar" = "minor"
"architect-dayplan" = "minor"
"architect-navigator" = "minor"
"architect-navigator-api" = "minor"
---

Added the Calendar section of the navigator's left list (#444): a provider's `Snapshot::calendar` (`CalendarGrid`, `GridDay`) draws a month grid under its top row (`NavRowKind::MonthGrid`, `Mount::calendar`, `SectionCalendar`, `ProviderSource::section_calendar`, `NavigatorState::section_grid`, `pick_section_day`, `section_move`, `section_today`), whose day click selects that day and opens its daily note and whose days take drops; the calendar at a pane's foot gives way to it. The Calendar's provider (`CalendarNav`) is now the periodic tree, Year → Quarter → Month → Week → Day, each day listing its events, tasks, tracked time, day plan and note by time, with events and tasks dragged onto a day moved there; it reads day plans from `architect.dayplan` (`architect_calendar::plan`, `refresh_plans`). Added `Item::icon`; day answers from Day Plans carry their template's `color`.
