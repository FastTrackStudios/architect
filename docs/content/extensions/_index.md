+++
title = "First-party extensions"
description = "The extensions architect ships (#368), each a model of how one is built: a linked extension that also runs as a process, its data as vault notes, its API as a service."
weight = 19
sort_by = "weight"
+++

Architect's own extensions live in this repo under `extensions/`, one
crate each, so they can be moved to a repo of their own later. They are
the app's features and the showcase of how an extension is built
([Extensions](@/architecture/extensions.md), "The rule"). Each one:

- is one `linked::Extension` value: the app links it in
  (`Extensions::with_linked`), and its crate's program serves the same
  value as a process (`Guest::extension`), so `EXT_CLONE {"id": …}` gives
  a development copy that overrides the built-in;
- keeps its data as plain markdown in the vault, reached only through the
  host service (`VAULT_*`, `BASES_*`), so the process copy works as the
  linked one does;
- offers its API as a service (`[provides]`), and lights up with the
  others it `[uses]` when they are installed, running alone when they are
  not;
- is tested both ways against a real vault.

| extension | what it is | page |
|---|---|---|
| Projects (`extensions/projects`) | a page on architect's built-in `project` kind, project types, goals and milestones | [Projects](@/extensions/projects.md) |
| Tasks (`extensions/tasks`) | tasks as vault notes in TaskNotes' shape: list, Today, My tasks, a kanban base view | [Tasks](@/extensions/tasks.md) |
| Time Tracker (`extensions/time-tracker`) | timers and time entries in a note's `timeEntries`, totals per task, project, day and week, a timesheet base view, a report | [Time Tracker](@/extensions/time-tracker.md) |
| Calendar (`extensions/calendar`) | events as notes (Task's shape) with repeats and exceptions, day, week, month and agenda views with drag, tasks and tracked time on it, calendar and timeline base views | [Calendar](@/extensions/calendar.md) |

Projects, Tasks and the Time Tracker are the Project Management bundle
(#375): each runs alone, and together a project's page shows its tasks
and their time, and a task its timer.
