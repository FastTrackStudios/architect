+++
title = "Day Plans"
description = "How the Day Plans extension is built: Task's day templates as vault notes, rules and overrides that pick each day's template, a past that never moves, each day's own plan, Task's resolver, and the architect.dayplan service the Calendar draws plans with."
weight = 23
+++

Day Plans (`extensions/dayplan`, crate `architect-dayplan`, id `dayplan`)
gives every day a shape. It brings Task's day templates
(`task:features/scheduling/scheduling-proto/src/time_block.rs`,
`day_plan.rs`, `resolve.rs`) to architect: reusable days of time blocks,
and each date's own copy with its blocks given to a task, a project, a
label or a recipe. The Calendar draws a day's plan under its events
(see [Calendar](@/extensions/calendar.md)).

## 1. A template is a note of blocks

A template is a reusable day: "Workday", "Studio day", "Rest day". Each
block has a start and an end (`HH:MM`; an end at or before the start
wraps past midnight, as sleep does), a label and a category:

| Category | What it is | How the resolver treats it |
|---|---|---|
| `reset`, `spiritual`, `meal`, `exercise`, `hygiene`, `wind_down`, `sleep` | the routine | anchored: shifts up to two hours, or shrinks |
| `allocatable` | Task's three Work / Event / Free-time blocks ("Block 1/2/3"), given to something day by day | flexible: shrinks in place |
| `maintenance`, `other` | errands, admin | flexible |

A block can be `fixed` ("at the studio 9:00"): it never moves.

```markdown
---
type: day-template
name: Workday
color: "#3b82f6"
blocks:
  - {id: reset, start: "06:30", end: "07:00", label: Morning reset, category: reset}
  - {id: block-1, start: "08:00", end: "11:30", label: Block 1, category: allocatable}
  - {id: lunch, start: "12:00", end: "12:45", label: Lunch prep, category: meal}
  - {id: sleep, start: "22:00", end: "06:30", label: Sleep, category: sleep}
---
```

Templates live in `Calendar/Day templates/` (the folder is the setting
`dayplan.folder`). They are written key by key, so prose a person adds
under the frontmatter stays. The model is `extensions/dayplan/src/model.rs`:
`DayTemplate`, `TimeBlock`, `TimeOfDay`, `BlockCategory` (its colours are
Task's), `PlannedBlock`, `BlockAssignment`, `DayPlan`. It is pure.

## 2. Rules pick a day's template, overrides beat them

`Calendar/Day rules.md` holds the rules, in order. The first that matches
a day wins:

```markdown
---
type: day-template-rules
rules:
  - {template: Gig day, tag: gig}                                   # a day with an event tagged #gig
  - {template: Studio day, rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=SA", start: 2026-10-31}
  - {template: Road day, from: 2026-12-01, to: 2026-12-07}          # a tour week
  - {template: Workday, weekdays: [mon, tue, wed, thu, fri]}
fallback: Rest day
overrides:
  - {from: 2026-11-12, to: 2026-11-12, span: day, template: Studio day}
---
```

- **Overrides** are chosen by hand, for a day, a week or a range of
  dates; `template: null` clears a day. A day's override beats a week's,
  and both beat every rule.
- A rule's tag is matched against the Calendar's events that day: an
  event's `tags`, or a `#tag` in its title. The Calendar's
  `CALENDAR_AGENDA` gives each event's `tags` for this.
- Every day knows why it has its template: `Rule 1: a day with #gig`,
  `Override for this day`, `No rule matched: the default`
  (`extensions/dayplan/src/rules.rs` `Rules::resolve`, `Reason::words`).

## 3. Only today and the future change

A change to the rules (`Rules::change`) or to a template's blocks
(`DayTemplate::change_blocks`) applies from today on. What they replace
is kept, until yesterday, in the note's `history`; a past day resolves
with the rules and blocks it had then (`Rules::in_effect`,
`DayTemplate::blocks_on`). The calendar's history never shifts, without
a file written for every day that passes. An override is the one thing
that changes a past day, to correct the record.

## 4. A day's own plan

A day with no plan of its own is its template's, so a change to the
template or the rules reaches it. The first edit (a block moved or
resized, a block given to something) saves the whole day as a plan,
`Calendar/Day plans/<date>.md`, with the template and the reason it had.
An edited day keeps its edits when templates and rules change;
`DAYPLAN_RESET` ("Reset to template") deletes the plan. A day's own
override starts its plan again from the template chosen.

Plans get a file of their own, not the daily note's frontmatter: a day is
planned without making its daily note, a plan is rewritten whole without
touching what is written in the daily note, and a reset is deleting a
file.

```markdown
---
type: day-plan
date: 2026-11-09
template: Workday
reason: "Rule 3: Mon–Fri"
blocks:
  - {id: block-1, start: "08:00", end: "11:30", label: Block 1, category: allocatable,
     assignment: {kind: project, title: Album, path: Projects/Album/project.md}}
  - {id: lunch, start: "12:00", end: "12:45", label: Lunch prep, category: meal,
     assignment: {kind: recipe, title: Lentil soup, path: "Resources/Recipes/Lentil soup.md"}}
---
```

`extensions/dayplan/src/book.rs` `Book::day` puts a day together: its plan,
else its template's blocks as they were that day. `assign` and
`set_span` edit it; `planned_vs_tracked` sets the Time Tracker's spans
against each block.

## 5. Task's resolver, as it is

`extensions/dayplan/src/resolve.rs` is Task's, ported unchanged with its
tests: fixed blocks keep their span; anchored routine blocks shift (up to
two hours) or shrink; a meal with no room left is embedded inside the
fixed block that covers it (lunch inside a work shift), at most thirty
minutes; flexible blocks shrink in place or drop under fifteen minutes.
Every change is reported. `DAYPLAN_UPSERT` runs it with `reconcile`, and
`merge_template` with `merge` (a sparse plan filled from its template).
Task's weekday/weekend pick is replaced by the rules.

## 6. Actions and the service

Its actions are `DAYPLAN_*`, and its service `architect.dayplan`:

- `DAYPLAN_GET` (Task's `get_day_plan`), `DAYPLAN_DAYS` (what the Calendar
  draws), `DAYPLAN_UPSERT` (Task's `upsert_day_plan`), `DAYPLAN_ASSIGN`,
  `DAYPLAN_SET_BLOCK`, `DAYPLAN_RESET`;
- `DAYPLAN_TEMPLATES`, `DAYPLAN_TEMPLATE` (make, change from today on,
  rename with the rules following), `DAYPLAN_SET_WEEKDAY`;
- `DAYPLAN_TEMPLATE_RULES` (list or set), `DAYPLAN_OVERRIDE` (a date, a
  week or a range; a template or none; `clear`), `DAYPLAN_RESOLVE` (a
  date's template and why).

It reaches notes only through the host (`extensions/dayplan/src/vault.rs`),
and the Calendar and the Time Tracker through their services when they are
there (`[uses]`): it runs alone, with no tags and no tracked time.

## Tests to read

- `extensions/dayplan/tests/model.rs`: the rules in order, RRULEs and
  ranges, overrides, a rule change that leaves yesterday as it was and
  reaches tomorrow, a template change that leaves an edited day alone,
  planned against tracked, and the notes read back.
- `extensions/dayplan/tests/vault.rs`: linked in beside the Calendar and
  the Time Tracker over a real vault, every action.
- `extensions/dayplan/src/resolve.rs`: Task's resolver tests.
