+++
title = "Projects"
description = "How the Projects extension is built: a page on architect's built-in project kind, project types through [[project_types]], goals and milestones as notes, and the architect.projects service."
weight = 10
+++

Projects (`extensions/projects`, crate `architect-projects`, id
`projects`) gives the vault's projects a page and actions, adds the
`software` and `client` project types, and keeps goals and milestones as
notes. This page walks through how it is built.

## 1. A project is architect's

Architect owns the `project` kind (#344): a folder whose `project.md` is
the project, with the kind's properties (`state`, `start`, `due`, `up`,
`members`, `icon`; `features/vault/vault-live/src/property_schema.rs`).
The extension adds no kind of its own for projects; it reads and writes
that one (`extensions/projects/src/model.rs` `Project`, `State`). A
project's notes are what is in its folder.

Goals and milestones are notes of their own kinds, ported from Task
(`task:features/goal/goal-proto/src/model.rs`,
`task:features/milestone/milestone-proto/src/model.rs`): `Goal` (`kind`
lifetime … weekly, `status` aspiration … abandoned, `parent`,
`targetDate`) and `Milestone` (`status` open or closed, `dueDate`,
`goal`). Each names its project by link, and one inside a project's
folder belongs to it (`belongs`). A task names its milestone in
`milestone`, so a milestone's progress is its tasks'.

## 2. Project types

`[[project_types]]` is the contribution point for types of the `project`
kind ([Extensions](@/architecture/extensions.md), "Project types"). This
extension's are in its manifest:

```toml
[[project_types]]
id = "software"
name = "Software"
icon = "code"
properties = [
    { key = "repo", name = "Repository", kind = "link" },
    { key = "stack", name = "Stack", kind = "list" },
]
template = "templates/software.md"
folders = ["Docs", "Releases"]
views = ["page"]
```

Its templates ship in the package (`Projects::files`, so the linked copy
has them too). `PROJECTS_CREATE {title, type}` asks the app's registry
(`EXT_PROJECT_TYPES`) for the type, so it makes a project of any type a
loaded extension adds, not only its own: `projectType`, the properties'
defaults, the template as the body (`{{title}}` filled), and a folder note
in each folder. Turn the extension off and its types leave the registry.

## 3. Through the host, as a service

Like Tasks, it reaches notes only through the host service
(`extensions/projects/src/vault.rs`): one `BASES_ROWS` query finds every
project, goal and milestone (`notes_base`, sorted by `Book::of`),
`VAULT_CREATE` and `VAULT_SET_PROPERTY` write, `VAULT_LIST` lists a
project's other notes. Its actions are its API (`[provides]
"architect.projects" = "0.1.0"`):

| action | does |
|---|---|
| `PROJECTS_LIST {state?, type?}` | the projects |
| `PROJECTS_READ {path}` | a project's page data: its type, roadmap, goals, tasks, time and notes |
| `PROJECTS_CREATE {title, type?, …}` | a new project |
| `PROJECTS_SET_STATE {path, state}` | planning, active, on hold, done |
| `PROJECTS_ADD_MEMBER`, `PROJECTS_SET_MEMBERS` | its members |
| `PROJECTS_RESOLVE {link}` | the project a link, path, folder or name names (what Tasks calls) |
| `PROJECTS_TYPES` | the registry's types |
| `PROJECTS_GOAL_CREATE`, `PROJECTS_GOALS` | goals |
| `PROJECTS_MILESTONE_CREATE`, `PROJECTS_MILESTONES`, `PROJECTS_MILESTONE_SET_STATUS` | the roadmap |
| `PROJECTS_OPEN {path?}` | a project's page, or the list |

It `[uses] "architect.tasks"`: with Tasks on, the page lists the
project's tasks (`for_project`), adds one (`add`) and finishes one
(`set_status`); with it off, the page says so and the rest works. It
`[uses] "architect.time"` the same way: with the
[Time Tracker](@/extensions/time-tracker.md) on, `PROJECTS_READ` carries
the project's `time` (`for_project`), and the page shows it and starts
timers; with it off, the page has no Time section.

## 4. Views

- `list` (`ListView`): the projects by state, and a field to start one of
  a type.
- `page` (`PageView`, `ProjectPage`): the project's state (a click sets
  it), members, its type's details, the roadmap (`MilestoneLine`: each
  milestone's progress and due date; a field adds one), goals, its time
  (`TimeSection`: in all, this week, today, billable and its amount, the
  running timer or a Start button), tasks (`TaskLine`, each with its time
  and a ▶ that times it) and its other notes. A linked view is given no pane state,
  so `PROJECTS_OPEN` sets the project the page shows before it opens it.

## 5. Tests

- `extensions/projects/src/model.rs`: projects, links, goals, milestones.
- `extensions/projects/tests/bundle.rs`: with Tasks over a real vault (a
  software project, its milestone's progress from its tasks, a task's
  project resolved by name), each alone (Tasks off: the page says so;
  Projects off: its types go, Tasks still files tasks), and as a process.

## Status and what's next

Done: the page, project types, goals and milestones, the service. Next: a
navigator provider (projects or areas on the left, a project's contents on
the right) when #356's API lands, a tour when `[[tours]]` (#379) lands,
and `PROJECTS_*` as architect's own actions when #344 adds them. The
project's time comes from the Time Tracker.
