+++
title = "Time Tracker"
description = "How the Time Tracker extension is built: timers and time entries as rows of a note's timeEntries (TaskNotes' shape), totals per task, project, day and week, a timesheet view type for Bases, a report, a status item, a capture target, and the architect.time service Projects and Tasks use."
weight = 21
+++

The Time Tracker (`extensions/time-tracker`, crate `architect-time-tracker`,
id `time`) times your work on tasks and projects and adds it up. It is
the third member of the Project Management bundle (#375), beside
[Projects](@/extensions/projects.md) and [Tasks](@/extensions/tasks.md).
This page walks through how it is built, in the order you would build one
like it.

## 1. An entry is a row of a note's `timeEntries`

Task keeps two kinds of tracked time: `task::TimeEntry`, an inline list in
a task note's frontmatter (`task:features/task/task-proto/src/model.rs`
`TimeEntry`), and `WorkSession`, a database row with a billable flag, a
rate and an org (`task:features/timer/timer-proto/src/session.rs`). An
extension reaches notes only, so the Time Tracker keeps the first, in
TaskNotes' shape, and carries what it can of the second:

```markdown
---
type: task
title: Mix the album
projects: ["[[Projects/Album/project|Album]]"]
timeEntries:
  - startTime: 2026-11-06T09:00:00Z
    endTime: 2026-11-06T10:30:00Z
    description: Rough mix
    billable: true
  - startTime: 2026-11-06T14:00:00Z
---
```

- An entry sits on the note the time was spent on: a task, a project's
  `project.md`, or `Time/Unfiled.md` for time on neither.
- The entry with no `endTime` is the running timer. One runs at a time:
  starting one stops the other, as Task's `switch_timer` does.
- Billing is SolidTime's, read off the project note with Task's keys:
  `billableDefault` (a new entry on the project or its tasks is written
  `billable: true`; Task snapshots it at start too), `defaultRateCents`
  and `currency`.
- Rows it cannot read, and keys it does not know, are written back as
  they were.

The model is `extensions/time-tracker/src/model.rs`: `Entry`
(`from_value`, `to_value`, `seconds`, `seconds_in`), `appended` and
`stopped` (the list with an entry added, or every running one ended),
`Tracked` (a note with time on it, `project_path`), `parse_duration` and
`log_line` (what a person types: `1h30m`, `45m`, `1:30`, `for:1h30`),
`Span` (today, week, last week, month), `running` and `report`. It is
pure: the clock and the time zone are arguments.

## 2. It reaches the vault only through the host

`extensions/time-tracker/src/vault.rs` is the only place that touches
notes, through `linked::Handle::host` with `HostRequest::Run`:

- `rows` finds every note with time on it, and every project, in one
  `BASES_ROWS` query (`tracked_base`: `file.hasProperty("timeEntries")`,
  `type == "project"` or `file.name == "project"`). Projects come along
  so their rates are known before their first entry.
- `properties` reads one note (`VAULT_READ`) right before it is written,
  so a write never works from a stale index.
- `set` writes the whole `timeEntries` list back (`VAULT_SET_PROPERTY`).

The manifest says so: `[capabilities] actions = ["VAULT_", "BASES_",
"EXT_VIEW_OPEN"]`. The process copy (`src/main.rs`,
`Guest::extension(&TimeTracker).serve()`) works as the linked one does.

## 3. Its actions are its API

Every action is a method of `architect.time` (`[provides]`): method
`start` is `TIME_START`.

| action | does |
|---|---|
| `TIME_START {task? \| project? \| note?, description?, billable?}` | a timer on the note (Time/Unfiled.md when none is named); a running one stops first |
| `TIME_STOP` | ends the running timer |
| `TIME_ACTIVE` | the running timer, or `null` |
| `TIME_TOGGLE` | its status item's press: stop, or open the report |
| `TIME_LOG {…, duration? \| minutes? \| start+end, date?}` | a finished entry by hand (Task's `log_session`) |
| `TIME_CAPTURE {title, body, properties, …}` | the `time` capture target's action (below) |
| `TIME_TOTALS {paths?}` | each note's seconds in all, today, this week, and whether it runs |
| `TIME_FOR_PROJECT {project}` | a project's time, its own and its tasks', billable seconds and the amount, the running timer, each task's |
| `TIME_REPORT {range? \| from+to}` | totals per day, per note and per project over a range |
| `TIME_OPEN {view?}`, `TIME_TIMESHEET` | open the report, or the timesheet |

## 4. Working with Tasks and Projects

It `[uses]` `architect.tasks` and `architect.projects`, and runs alone
without them:

- **Naming:** `TIME_START {task: "[[Mix the album]]"}` finds the task
  through Tasks' `read`, and `{project: "Album"}` the project through
  Projects' `resolve` (`target` in `extensions/time-tracker/src/lib.rs`).
  Alone, a path or a link with a folder works as it is.
- **A task's time counts toward its project** (`projects_of`): its
  `projects` link with a folder, else the project Projects resolves a bare
  link to, else the project whose folder it sits in. `TIME_FOR_PROJECT`
  adds the tasks Tasks' `for_project` names.
- **The other way round**, Projects and Tasks use `architect.time`: the
  project page's Time section (`extensions/projects/src/lib.rs`
  `TimeSection`: in all, this week, today, billable and its amount, the
  running timer, a Start button) and its task lines' time and ▶ button
  (`TaskLine`); the Tasks list's rows (`extensions/tasks/src/lib.rs`
  `TaskRow`) and each task's `time` in `TASKS_LIST`, `TASKS_READ` and
  `TASKS_FOR_PROJECT` (`refresh_time`). Without the Time Tracker they
  show what they showed before.

## 5. Views and the status item

- `report` (`ReportView`): range chips (today, this week, last week, this
  month, all), the running timer with its clock and Stop, a "Log time"
  field (`45m Call with the label [[Album]]`), the totals and amounts,
  then by day, by project and by task or note, each row's ▶ timing it.
- `timesheet`, a view type for Bases (`[[base_views]] id = "timesheet"`,
  `Builder::base_view("timesheet", Timesheet)`): a week, a row per note
  with time in it and a column per day, totals on the right and at the
  bottom; ‹ and › page through weeks. `TIME_TIMESHEET` writes
  `Time/Timesheet.base` once and opens it, and any base can show its
  notes as a timesheet.
- The status item (`[[status]] id = "timer"`, `status_text`) says what is
  running and since when (`Mix the album · since 09:00`, beside its
  `timer` icon), or today's
  total; a press runs `TIME_TOGGLE`. It shows a start time rather than a
  ticking clock, so it stays true between reads.
- The navigator (`extensions/time-tracker/src/nav.rs` `TimeNav`, while the
  report shows): Running, Today, This week and Earlier, each note in the
  one it was last worked on; drag one onto Running to time it.

## 6. A capture target: "Log time"

`[[capture_targets]] id = "time"` puts "Log time" in architect's composer
and inbox deck, with a `for:` token: `Call with the label for:45m
[[Album]] today` logs 45 minutes on Album today, described as "Call with
the label" (`capture`). A length typed in the title (`45m`) works too.

## 7. Tests

- `extensions/time-tracker/src/model.rs` and
  `extensions/time-tracker/src/nav.rs`: entries, durations, spans and
  reports (across local midnight in a fixed time zone), the navigator's
  groups; pure.
- `extensions/time-tracker/tests/vault.rs`: the extension in a host whose
  app list has the real `VAULT_*`, `BASES_*` and `CAPTURE_*` actions over
  a folder, linked in and then as its process, every entry checked in the
  markdown; then with Tasks and Projects, names resolved through their
  services and the project page's and task list's time read through
  `architect.time`.

## Status and what's next

Done: timers and entries, totals per task, project, day and week,
billable time and amounts from the project's rate, the report, the
timesheet view, the status item, the navigator, the "Log time" target,
the service, and its use by Projects and Tasks. Next: per-member rates
(Task's `ProjectMemberRate`, `OrgMemberRate`) when orgs carry members,
editing and deleting an entry from the report, reporting to Finance
(#370) when it is installed, a bundle manifest when #367 lands, and the
same in Studio.
