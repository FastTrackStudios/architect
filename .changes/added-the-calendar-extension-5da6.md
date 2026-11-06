---
"architect-app" = "minor"
"architect-calendar" = "minor"
"architect-navigator" = "minor"
"architect-navigator-api" = "minor"
"architect-navigator-extension" = "minor"
"architect-tasks" = "minor"
"architect-time-tracker" = "minor"
---

Added the Calendar extension (architect-calendar, id calendar, #336, #350): events as vault notes in Task's shape, repeats (RRULE) with exceptions (exdates and override notes), day, week, month and agenda views with drag to move and resize, tasks on their days through architect.tasks and tracked time through architect.time (new TIME_ENTRIES), calendar and timeline base view types, the event capture target, a navigator provider, a tour, and the architect.calendar service (CALENDAR_*). A navigator provider can keep its nodes' order (Snapshot::ordered), and a provider's NavigatorMount hands periodic-note actions to the vault's navigator (on_vault).
