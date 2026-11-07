+++
title = "Calendar"
description = "How the Calendar extension is built: events as vault notes in Task's shape, repeats with exceptions, day, week, month and agenda views with drag to move and resize, tasks and tracked time on it, calendar and timeline view types for Bases, a capture target, a navigator provider, and the architect.calendar service."
weight = 22
+++

The Calendar (`extensions/calendar`, crate `architect-calendar`, id
`calendar`) is Task's calendar (`task:features/scheduling`,
`task:features/view/view-calendar`) as an extension. It runs alone. With
[Tasks](@/extensions/tasks.md) and the [Time Tracker](@/extensions/time-tracker.md)
it also shows your tasks on their days and your tracked time. With
Navigator, a day's heading opens its daily note (#336). This page walks
through how it is built.

## 1. An event is a note

Task keeps events as notes under `Records/events`. The Calendar reads them
wherever they are, by type, and writes new ones to its folder
(`calendar.folder`, `Calendar` by default):

```markdown
---
type: scheduling-cal-event
id: album-release-show
title: Album release — show
start: 2026-12-05T20:00:00-06:00
end: 2026-12-05T22:30:00-06:00
location: The Hall
for:
  - '[[Gigs/album-release]]'
---
Load-in at four, soundcheck at six.
```

- `start` is inclusive and `end` exclusive, in RFC 3339 with the offset
  they were made in. An all-day event's are dates, and its `end` is the
  day after its last day.
- Any other property that is a wikilink, or a list of them, links the
  event to that note under its key (`for`, `with`). `CALENDAR_LIST
  {linked_to}` finds them.
- Task's `type: event` reads too. A note with no `id`, `title` or `color`
  falls back to its file name, its file name and `primary`
  (`extensions/calendar/src/model.rs` `Event::from_values`).
- The vault writes a new note's keys in alphabetical order; a rewrite keeps
  the note's own order, its body and any foreign key.

## 2. Repeats, and their exceptions

`recurrence` is an RFC 5545 RRULE. `extensions/calendar/src/recur.rs`
`Rule` reads the subset calendars write: `FREQ` (daily to yearly),
`INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (with ordinals: `-1FR`),
`BYMONTHDAY`, `BYMONTH`, `BYSETPOS` and `WKST`. It expands in the person's
time zone, so a 09:00 standup stays at 09:00 across a clock change. A rule
it cannot read shows the first occurrence alone, as Task's does.

Task's calendar has no exceptions: an occurrence is the master moved, and
dragging one moves them all. Here a series has two kinds, RFC 5545's
`EXDATE` and `RECURRENCE-ID`:

- `exdates`: the days of skipped occurrences (`CALENDAR_SKIP`).
- an override: a note of its own, with `recurrence_id` (the series' `id`)
  and `recurrence_date` (the day it replaces). Moving or changing one
  occurrence writes one (`CALENDAR_RESCHEDULE {occurrence, scope: "this"}`).

`scope: "following"` ends the series the day before (`UNTIL`) and starts
a new one there. `scope: "series"` moves every occurrence by as much as
the one dragged.

## 3. What the calendar draws

`model::occurrences` turns the events into blocks over a window. Tasks
come from `architect.tasks` (`list`): a task sits on its `scheduled` day,
or else its `due` day. With a time it is a block of its `timeEstimate` (30
minutes without one); without a time it sits in the all-day strip.
Tracked time comes from `architect.time` (`entries`, `TIME_ENTRIES`), as
striped blocks. Either can be turned off (`calendar.show_tasks`,
`calendar.show_time`).

`extensions/calendar/src/layout.rs` places blocks the way Task's
view-calendar does:

- `day_columns`: a day's overlapping blocks split its width.
- `tracks`: multi-day and all-day blocks run across a row.
- `days` and `step`: what day, week, month (six weeks) and agenda (30
  days) show.

## 4. The views

`ui::CalendarView` holds the toolbar (‹ Today ›, Day/Week/Month/Agenda,
quick add, + New), the grid and the editor:

- **Week and day:** drag a block to move it by 15 minutes and across days.
  Drag its top or bottom edge to resize it, or sweep over free time to make
  one. A click opens the editor. An occurrence moves alone, and the editor
  can change this one, this and following, or all.
- **Month:** drag a block to another day. A click on a free part of a day
  makes an event there. A day's number opens its daily note.
- **Agenda:** the next 30 days as a list.
- **Keys:** `t`, `1`–`4`, `n`, ← →.

The drags follow the navigator's pattern: a press is noted, a move past a
few points makes it a drag, and the grid's measured box turns the pointer
into minutes and days.

It adds two view types to Bases (`[[base_views]]`): `calendar`, a month of
a base's notes by a date property (`startBy`, `endBy`), and `timeline`, a
schedule of them as bars. Drag a bar to move it, or its end to change the
end. `CALENDAR_BASE` writes and opens `Calendar/Calendar.base` or
`Calendar/Timeline.base`.

## 5. Daily notes

The navigator owns the periodic notes, and that code is GPL. A day's
heading runs `NAVIGATOR_OPEN_DAILY_NOTE` (`CALENDAR_DAILY`), which is in
the Calendar's `[capabilities]`. The navigator makes or opens the note
with its own pattern and folder, so the Calendar never links the GPL
crate. While the calendar's own sidebar is on show, the navigator there is
a provider's. The mount's `on_vault` (`NavigatorMount`) switches the
sidebar to the vault, whose navigator does it.

## 6. Its API, and the rest

| action | what |
|---|---|
| `CALENDAR_LIST` | Task's `list_events`: the notes, by window, link, ref |
| `CALENDAR_AGENDA` | what shows over a window: occurrences, tasks, time |
| `CALENDAR_CREATE` | Task's `create_event`; an `external_ref` already used answers that one |
| `CALENDAR_UPDATE` | Task's `update_event`: never moves it; `tombstone` |
| `CALENDAR_RESCHEDULE` | Task's `reschedule_event`, with `occurrence` and `scope` |
| `CALENDAR_SKIP`, `CALENDAR_CANCEL` | one occurrence, or the note (and a series' overrides) |
| `CALENDAR_QUICK`, `CALENDAR_CAPTURE` | an event from a line: `Lunch with Sam tomorrow 1pm for 90m` |
| `CALENDAR_MOVE_TASK` | a task dragged: its scheduled (or due) date through `architect.tasks` |
| `CALENDAR_OPEN`, `CALENDAR_SHOWN`, `CALENDAR_DAILY`, `CALENDAR_BASE` | the views |

Each method `m` of the service `architect.calendar` is `CALENDAR_<M>`.
It also has:

- **The `event` capture target.** Its day comes from capture's parser, and
  its time and length from the title (`quick.rs`: `1pm`, `2-3pm`, `for
  45m`) or from the `at:` and `for:` tokens.
- **The navigator's Calendar section** (`nav.rs` `CalendarNav`, #444), as
  Notebook Navigator draws a calendar. Under the Calendar row in the left
  list, a month grid (`Snapshot::calendar`: each day's dots and its day
  template's mark) and the periodic tree, Year → Quarter → Month → Week →
  Day (`nav.rs` `tree`, `window`: last month to the month after next; a
  week is the month its fourth day is in). A period selected lists its
  days' events, tasks due or scheduled, tracked time, day plan and the
  period's note, by day and by time (`snapshot_of`). A click on a day in
  the grid selects it and opens its daily note; an event or a task dropped
  on a day, in the tree or the grid, moves there (`move_items`). There is
  no rail button: the section is the way in.
- **Day plans** (`plan.rs`, #444): each day's plan from the Day Plans
  extension's `architect.dayplan` `days`, when it is there (see
  [Day Plans](@/extensions/day-plans.md)). `refresh_plans` reads them
  outside `refresh`, as Day Plans reads the agenda for a rule's `#tag`.
- **A settings section** (`calendar.*`).
- **A tour** (`tours/calendar-tour.styx`).

Its tests (`extensions/calendar/tests/vault.rs`) run it linked in and as
a process against a real vault, then with Tasks and the Time Tracker.
Run them with `cargo nextest`, one process per test: the extension keeps
its state in process-wide statics.

## 7. Booking

Task's scheduling also books time (`task:features/scheduling/scheduling/src/slots.rs`).
`extensions/calendar/src/booking.rs` ports it with three kinds of note,
each in Task's shape:

- **An event type** (`scheduling-event-type`, `Calendar/Event types`): its
  length, its buffer, the schedule it is booked in, where it happens, and
  two keys Task's notes don't have, `notice_min` and `days_ahead`.
- **A schedule** (`scheduling-schedule`, `Calendar/Schedules`): its weekly
  hours, written as lines (`mon-fri 09:00-17:00`) or as Task's `{days,
  start, end}`.
- **A booking** (`scheduling-booking`, `Calendar/Bookings`): its start and
  end in UTC, the attendee, and a status.

`open_slots` steps through a schedule's hours by the length plus the
buffer, as Task's does. It also goes past Task's first cut in four ways:

- Bookings that hold their time block a slot, and so do the calendar's
  own events, each padded by the buffer.
- `notice_min` and `days_ahead` clamp the window.
- `CALENDAR_BOOK` refuses a slot that is no longer open.
- Bookings show on the calendar.

| action | what |
|---|---|
| `CALENDAR_SCHEDULE`, `CALENDAR_EVENT_TYPE` | make or change one |
| `CALENDAR_EVENT_TYPES` | the event types and schedules |
| `CALENDAR_OPEN_SLOTS` | Task's `list_open_slots` |
| `CALENDAR_BOOK` | Task's `book_slot` |
| `CALENDAR_BOOKINGS`, `CALENDAR_BOOKING_STATUS` | list them; Task's `update_booking_status` |
| `CALENDAR_BOOKINGS_OPEN` | the Bookings view: each type's slots this week, a click books one |

A schedule's `timezone` is kept, but not converted: its hours are the
person's time zone's. Task's audit log of booking changes
(`Records/audit/booking-events.jsonl`) is not ported.

## 8. Calendars from elsewhere

Task has no external calendars. The Calendar reads iCalendar feeds, read
only: a CalDAV calendar's `.ics` export, Google's or iCloud's secret
address, a `webcal://` link (`extensions/calendar/src/ics.rs`,
`extensions/calendar/src/sync.rs`).

- **A subscription is a note:** `Calendar/Subscriptions/<name>.md`, of
  type `calendar-subscription`, holding the feed's `url`, a `color` and
  when it was last `synced`. It syncs with the vault like any note.
- **A sync** fetches the feed through the host (`fetch`, which needs
  `capabilities.hosts = ["*"]` because the address is the person's). Each
  `VEVENT` becomes an event note in `Calendar/<name>/`, keyed by
  `external_ref` `ics:<UID>`:
  - repeats keep their `RRULE` and `EXDATE`s;
  - a moved occurrence (`RECURRENCE-ID`) is an override of its series;
  - `STATUS:CANCELLED` is a tombstone, and so is an event the feed no
    longer has.
  - A changed note is changed key by key, so its body and its own keys
    stay.
- **On the calendar they don't drag or edit.** The feed is where they
  change. A sync runs at each start, and on `CALENDAR_SYNC`.
- **Times:** `…Z` is UTC. A `TZID` or a floating time is read in the
  person's own time zone, since no `VTIMEZONE` is read.

| action | what |
|---|---|
| `CALENDAR_SUBSCRIBE` | a feed's `url`, `name`, `color`; synced at once |
| `CALENDAR_SYNC` | every subscription, or one |
| `CALENDAR_IMPORT` | an `.ics` file's text, once (again with the same name: updated) |
| `CALENDAR_SUBSCRIPTIONS`, `CALENDAR_UNSUBSCRIBE` | list them; remove one and its notes |

Two-way CalDAV, where edits here go back to the server, is not built.
Day-plan templates are the Day Plans extension's
([Day Plans](@/extensions/day-plans.md)).
