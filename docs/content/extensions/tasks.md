+++
title = "Tasks"
description = "How the Tasks extension is built: tasks as vault notes in TaskNotes' shape, reached through the host service; its list, Today, My tasks and a kanban view type for Bases; the architect.tasks service."
weight = 20
+++

Tasks (`extensions/tasks`, crate `architect-tasks`, id `tasks`) keeps your
tasks as notes in the vault and gives them a list, Today, My tasks and a
board. This page walks through how it is built, in the order you would
build one like it.

## 1. A task is a note

A task is a markdown note with `type: task` (or TaskNotes' `task` tag), its
fields in the frontmatter under TaskNotes' names, so a TaskNotes vault
reads as it is:

```markdown
---
type: task
title: Write the copy deck
status: open
priority: high
due: 2026-11-05
tags: [task, content]
projects: ["[[Projects/Website Relaunch/project|Website Relaunch]]"]
blockedBy: ["[[Projects/Website Relaunch/Tasks/Pick a hosting plan]]"]
recurrence: FREQ=WEEKLY
recurrence_anchor: completion
---
```

The model is a port of Task's `TaskInfo` (`task:features/task/task-proto/src/model.rs`)
in `extensions/tasks/src/model.rs`: `Task` (`from_values`, `frontmatter`,
`in_project`, `assigned_to`), the status classification (`Status`,
`Group`, `status_is_terminal`: one terminal set, so `shipped` and
`archived` close a task everywhere), `Priority`, the due rule
(`is_due_on_or_before`, `is_today`), the list order (`order`), repeats
(`Repeat`, a subset of RRULE, and `complete`: a repeating task done stays
open, its date moved on and the day added to `complete_instances`) and
relations (`edges`, `relations_of`: `blockedBy`, `parent` for sub-tasks,
`relates`).

Where Task points by UUID, a note points by link. A link is what Obsidian,
the vault's index and a person reading the file all follow, and it
survives a rename (`VAULT_RENAME` rewrites links).

## 2. It reaches the vault only through the host

`extensions/tasks/src/vault.rs` is the only place that touches notes, and
it does so through `linked::Handle::host` with `HostRequest::Run`:

- `rows` finds every task in one query, `BASES_ROWS` with a base written
  as YAML (`tasks_base`: `type == "task"` or `file.hasTag("task")`);
- `create` writes a note with `VAULT_CREATE` (a taken name gets ` 2`);
- `set` changes one property with `VAULT_SET_PROPERTY` (or removes it).

The manifest says so: `[capabilities] actions = ["VAULT_", "BASES_",
"EXT_VIEW_OPEN"]`. Because nothing reaches for the app's Rust handles, the
process copy (`src/main.rs`, `Guest::extension(&Tasks).serve()`) works
exactly as the linked one.

## 3. Its actions are its API

Each action is in `extensions/tasks/extension.toml` (`[[actions]]`, with
its JSON schema) and built in `Tasks::build`:

| action | does |
|---|---|
| `TASKS_LIST {view?, project?}` | all, open, today or mine (assigned to Settings → Tasks → Your name) |
| `TASKS_READ {path}` | one task and its relations |
| `TASKS_ADD {text, project?}` | quick add (below) |
| `TASKS_CAPTURE {title, body, properties, …}` | the `task` capture target's action (below) |
| `TASKS_CREATE {title, …}` | a task with the properties given |
| `TASKS_UPDATE {path, …}` | sets properties (`null` removes one) |
| `TASKS_SET_STATUS {path, status}` | a status; done stamps `completedDate`, a repeat moves on |
| `TASKS_FOR_PROJECT {project}` | a project's tasks, how many are done, per milestone |
| `TASKS_STATUSES` | the status groups |
| `TASKS_OPEN {view?}`, `TASKS_BOARD` | open a view, or the board |

`[provides] "architect.tasks" = "0.1.0"` makes them a service: method `m`
is `TASKS_<M>`, so another extension calls `Handle::service("architect.tasks",
"for_project", …)` (a page: `services.call`), which is how the project page
lists a project's tasks.

## 4. Quick add

Quick add reads words through architect's one parser
(`features/capture/src/parse.rs` `parse`, Task's capture parser with the
same rules). `Call the venue tomorrow [[Album]] #gigs @phone !high every week`
becomes a title, a project link, a tag, a context, a priority, a due date and
a repeat. The list's "New task" field and `TASKS_ADD` go through
`extensions/tasks/src/quick.rs` `parse`.

Tasks is a capture target too (`[[capture_targets]] id = "task"`,
[Capture](@/features/capture.md)). The composer and the inbox deck offer
"Task", and `TASKS_CAPTURE` makes one from a capture
(`extensions/tasks/src/quick.rs` `from_capture`): its properties become the
task's, and its body becomes the task's details.

## 5. Belonging to a project

A task's project is a link to a `project.md`. `TASKS_ADD` resolves
`[[Album]]` through the Projects extension's service
(`[uses] "architect.projects" = "^0.1"`, `resolve`) and writes the task
into the project's `Tasks/` folder with a link by path. `[uses]` is the
optional `[requires]`: with Projects off, Tasks still loads, and a path to
a `project.md` still works. A task inside a project's folder belongs to it
too (`Task::in_project`).

## 6. Views

- `list`, `today`, `mine` (`TaskList`): one component, three filters
  (`select`, `Filter`); a "New task" field, then a section per status
  group, each row's box finishing it (`TaskRow`). It reads again every
  few seconds while it shows (`use_tasks`), so an edit in the note shows
  up.
- `kanban`, a view type for Bases (`[[base_views]] id = "kanban"`,
  `Builder::base_view("kanban", Kanban)`): a column per value of the
  view's `columnBy` (status by default, its columns the status groups), a
  card's arrows moving it on (through `set_status`, so a repeat moves on).
  `TASKS_BOARD` writes `Tasks/Board.base` once and opens it.

A status item counts what is due today; a startup hook fills it.

## 7. Tests

- `extensions/tasks/src/model.rs` and `extensions/tasks/src/quick.rs`:
  the model and the parser, pure.
- `extensions/tasks/tests/vault.rs`: the extension in a host whose app
  list has the real `VAULT_*` and `BASES_*` actions over a folder, linked
  in and then as its process; every change checked in the markdown.

## Status and what's next

Done: the model, quick add, the views, the board, the service, the `task`
capture target. Next: a navigator provider (projects on
the left, a project's tasks on the right) when #356's API lands, a tour
when `[[tours]]` (#379) lands, and timers from the Time Tracker (#375).
