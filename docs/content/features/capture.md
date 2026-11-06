+++
title = "Capture"
description = "architect-capture: one quick-add parser, the composer, a capture window on a system-wide shortcut, the vault inbox and processing it, capture targets extensions add, CAPTURE_* and INBOX_*."
weight = 57
+++

## What it is and when to use it

Every app needs to capture things, so capture is architect's (#352), ported
from Task. A line goes in from the composer, the capture window, ⌘K, a menu,
the menu-bar item or an agent (`CAPTURE_ADD`). The **quick-add parser** reads
it: `[[Project]]`, `#tag`, `@context`, `!high`, `today` / `tomorrow` / a
weekday / `next fri` / `YYYY-MM-DD`, and `every week` / `every 3 days` /
`every weekday` become properties, and the rest becomes the title. It lands
in the vault's **inbox** as a note. Each capture comes back once, in the
**inbox deck** or through `INBOX_*`, and leaves in one of four ways:

- made into a **target's** thing (a task, …);
- filed as a plain note;
- put off until a later day;
- discarded.

Extensions add what a capture can become with `[[capture_targets]]`. The
Tasks extension's `task` is the first
([Extensions: capture targets](@/architecture/extensions.md#capture-targets)).

A capture's note keeps Task's inbox frontmatter, so a vault Task wrote reads
the same. The demo vault's convention (`type: capture`, `captured`,
`processed: false`) is read too:

```markdown
---
type: inbox-item
status: open
source: window
created: 2026-11-05T23:00:21-08:00
due: 2026-11-06
priority: high
projects: ["[[Lab]]"]
tags: [design]
---
The lines typed after the first.
```

## A minimal example

```rust
use architect_capture::actions::{self, Config};

// The list already has the vault's VAULT_* (and, with extensions, EXT_CAPTURE_TARGETS).
let capturer = actions::register(&list, Config::default(), None)?;
list.run_json_async("CAPTURE_ADD", br#"{"text":"Call the venue tomorrow !high"}"#).await?;
```

With a window, `architect_capture::ui::use_capture(&list, Config::default())`
registers the same actions, with `CAPTURE_OPEN` and `INBOX_OPEN` opening the
composer and the deck. Mount `CaptureLayer { capture }` once over the window.

## Code tour

- `features/capture/src/parse.rs` holds the parser. `parse` / `parse_with`
  return a `Draft`: the title, tags, contexts, projects, priority, due date,
  `Repeat` and extensions' `extra`. `Draft::properties` gives `TaskNotes`'
  keys (`keys`). `merge` lays hand-set chips over the parsed properties (a
  chip set by hand beats the text, and a hand-set `null` removes one).
  `TokenRule` is an extension's word, like Keyflow's `key:D`.
- `features/capture/src/inbox.rs` covers inbox notes. `note_for` builds a
  capture's note (the first line parsed, the rest the body). `Item::from_note`
  reads a `VAULT_READ` answer back. `Item::is_due` handles `resurface_on`.
  `carried` drops the inbox's own keys when a capture leaves. `suggests_task`
  is Task's `kind_from_text`.
- `features/capture/src/target.rs`: `Target` (the app's `note`, then the
  registry's), and `Capture`, what a target's action is given.
- `features/capture/src/actions.rs` (feature `actions`) has `Capturer`, the
  one path behind the actions, the composer and the deck. It writes only
  through `VAULT_*` and runs each as the caller. `register` is the entry
  point, with `ids` and `Opener`.
- `features/capture/src/ui.rs` (feature `ui`) has `use_capture` /
  `CaptureUi`, `Composer`, `InboxDeck` and `CaptureLayer`. The capture window
  is an `architect_workspace::Popout::page`, falling back to a card where
  there is no OS window.
- `features/capture/src/shortcut.rs` (feature `native`, desk only) has
  `use_global_shortcut` (`global-hotkey`, registered on the event loop's
  thread as macOS requires) and `use_float_window` (the window over every
  app, a third of the way down the screen).
- `apps/ui/src/capture.rs` wires it into the Architect app:
  `use_app_capture`, ⌃⌥N by default (`ARCHITECT_CAPTURE_SHORTCUT` names
  another), and `WINDOW_COMMAND` for the menu-bar item.
  `apps/ui/src/lib.rs` puts ⇧⌘N (`LAB_CAPTURE`), the feather and File ›
  Process inbox on it.

| action | what it does |
|---|---|
| `CAPTURE_OPEN {text?, target?, window?}` | opens the composer, or with `window` the capture window |
| `CAPTURE_ADD {text, properties?, target?, source?}` | captures into the inbox, or straight into a target |
| `CAPTURE_PARSE {text}` | the title, properties and suggested target, writing nothing |
| `CAPTURE_TARGETS` | the app's note, then every `[[capture_targets]]` |
| `INBOX_OPEN` | opens the deck |
| `INBOX_LIST {all?}` | the captures waiting, oldest first |
| `INBOX_PROCESS {path, target, title?, properties?}` | makes it `target`'s thing; it leaves the inbox |
| `INBOX_FILE {path, folder?}` | files it as a plain note |
| `INBOX_DEFER {path, until?}` | puts it off (a date or a quick-add day; tomorrow when unset) |
| `INBOX_DISCARD {path}` | deletes it (destructive: an agent's call waits for a yes) |

An `INBOX_*` only reaches notes directly in the inbox folder. When a capture
becomes a target's thing, its inbox note is removed through the app's door:
that removal is the move's other half, not a deletion the caller asked for.

## Tests to read

- `features/capture/src/parse.rs`, `features/capture/src/inbox.rs`,
  `features/capture/src/target.rs`: the parser (the issue's
  `[[Lab]] tomorrow !high #design`), hand-set chips, extension tokens, and
  reading notes back.
- `features/capture/tests/inbox.rs`: the actions over a real vault folder
  with a made-up target. It covers capture, list, process, file, defer and
  discard, each checked in the markdown.
- `extensions/tasks/tests/vault.rs`: the real Tasks extension, linked in and
  as its process. An inbox capture processed into a task is checked on disk.
- `features/extensions/tests/interop.rs`
  `capture_targets_are_registered_while_loaded`.

## Status

Done:

- the parser, shared with Tasks (`TASKS_ADD`);
- the inbox and its actions;
- `[[capture_targets]]` and the `task` target;
- the composer and the deck;
- the capture window on a system-wide shortcut, from the menu-bar item and
  `CAPTURE_OPEN`.

Next:

- inline pickers for `@` and `#`, and a project picker from the vault's
  projects;
- the phone's sheet with the chips on the Typing Toolbar;
- the Board's target in the sandbox;
- daily-note and project targets;
- the shortcut as a setting;
- the iOS share sheet and widget.
