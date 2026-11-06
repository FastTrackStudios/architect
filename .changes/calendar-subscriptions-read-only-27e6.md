---
"architect-calendar" = "minor"
---

Calendar subscriptions: read-only iCalendar feeds (https://, webcal://; CalDAV exports, Google and iCloud addresses) and .ics imports as event notes keyed by external_ref ics:<UID>: RRULE and EXDATE kept, RECURRENCE-ID as overrides, cancelled and dropped events tombstoned, synced at start and by CALENDAR_SYNC; read only on the calendar. CALENDAR_SUBSCRIBE, CALENDAR_SYNC, CALENDAR_IMPORT, CALENDAR_SUBSCRIPTIONS, CALENDAR_UNSUBSCRIBE.
