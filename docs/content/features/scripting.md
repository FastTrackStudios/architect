+++
title = "Scripting"
description = "architect-scripting: Luau over the Action List; a run is one undo step; saved scripts become actions in ⌘K, keys and MCP; errors carry their line."
weight = 50
+++

## What it is and when to use it

A script is a Luau program over the app's Action List. It can list,
describe and run actions, and branch or loop between them. A run is one
named undo step, and a failed run is taken back. Each run gets a fresh
sandboxed state with memory and step limits, and every action it runs is
authorized for whoever ran the script. A script saved under a name becomes
an action of its own (`SCRIPT_MY_<NAME>` for the person, `SCRIPT_ORG_<NAME>`
for the organization), so it shows in ⌘K, can be bound to a key and is
reachable over MCP. A live script `defer`s functions that run once per frame;
other crates add globals (`ImGui`, `Canvas`) as modules.

Use it for a person's macros and conditional actions, and for a model's
one-off scripts over MCP. Design: [Extensions](@/architecture/extensions.md),
the sections on actions, the model and scripts.

Example: `examples/scripting`, a task board scripted from Luau, saved scripts as actions and the board over MCP; walked through in [Script an app, end to end](@/getting-started/scripting.md).

<!-- media: script-saved-in-palette — a saved script "Arm selected" found in ⌘K, run, and taken back with one undo -->

## A minimal example

`architect` with `action-list`; `architect-scripting` with `luau` (the runtime
and the `SCRIPT_*` actions), `crdt` (runs over a `History`) or `settings` (a
settings file as the library). After `features/scripting/tests/scripts.rs`:

```rust
use architect::action::{ActionCaller, ActionList};
use architect_scripting::actions::Scripting;
use architect_scripting::history::HistoryTransactions;
use architect_scripting::{Library, MemoryStore, Script};

let list = ActionList::new();                    // the app's actions registered on it
let scripting = Scripting::new(list.clone(), Library::new(MemoryStore::new()).with_org(MemoryStore::new()))
    .with_transactions(HistoryTransactions::new(doc.clone(), Arc::clone(&history)));
scripting.install()?;                            // SCRIPT_RUN, SCRIPT_SAVE, .. and the saved ones

let id = scripting.save(Script::new("Arm selected", r#"
    local selected = state.query("TRACKS_SELECTED")
    for _, id in selected do
      actions.run("TRACKS_ARM", { id = id, on = true })
    end
    return #selected
"#))?;                                           // "SCRIPT_MY_ARM_SELECTED"

let report = scripting
    .run(&ActionCaller::app(), "return #actions.list()", serde_json::Value::Null, "Count")
    .await;
// report.result: Ok(value) or a ScriptError with its kind, line and message
```

A live script without the `Scripting` layer (from `features/scripting/tests/live.rs`):

```rust
let (report, live) = Runtime::new(list).with_module(Arc::new(my_module))
    .start(&caller, src, args, "Tool").await;
let frame = live.unwrap().frame().await;          // runs what the script deferred
```

## Code tour

1. `features/scripting/src/lib.rs` `Script`, `action_id`, `Library`,
   `ScriptStore`, `MemoryStore`: scripts and where saved ones are kept.
2. `features/scripting/src/actions.rs` `Scripting` (`install`, `run`, `save`,
   `delete`, `describe`): the `SCRIPT_*` actions and one action per saved
   script; `MAX_RUNNING` stops a script running itself without end.
3. `features/scripting/src/luau.rs` `Runtime` (`with_module`, `with_gate`,
   `with_transactions`, `with_limits`, `run`, `start`): one sandboxed state per
   run, with the `actions`, `state`, `print`, `args`, `require` and `defer` globals.
4. `features/scripting/src/luau.rs` `Limits`, `Report`: 64 MiB and 10 million
   steps by default; the result, log, actions run and undo step.
5. `features/scripting/src/luau.rs` `Live` (`frame`, `stop`), `Module`,
   `raise`: live scripts and the globals other crates add.
6. `features/scripting/src/history.rs` `HistoryTransactions`: a run as one
   `crdt::History` step.

`SCRIPT_RUN`, or a saved script's own action, reaches `Scripting::run`, which
counts running scripts and calls `Runtime::run`. The runtime builds a
sandboxed state and opens a transaction for the step. Each `actions.run` in
the script goes through `ActionList::run_json_async_with` with the original
caller, so permissions hold. On success the step commits; on failure it
aborts and the `ScriptError` carries the line that failed. The `Report` goes
back as JSON with the result, the log, the actions run and the undo step.

## Tests to read

- `features/scripting/tests/scripts.rs`: a saved script runs from ⌘K and undoes
  as one step; a model describes, runs and saves scripts over MCP; a failure
  reports its line and is rolled back; step and memory limits and the sandbox
  hold; a script runs with its caller's permissions; saving again replaces,
  deleting removes the action.
- `features/scripting/tests/live.rs`: the `defer` loop one frame at a time; a
  custom module; a failing frame stops the script; each frame is its own
  undo step; a one-off run cannot `defer`.

## Status

Done in architect; the lab uses it. Not yet: dry runs, the typed model
services in the Luau library, streams, Luau on the web.

- **Task** and **Studio** do not use it yet; both have Action Lists to run
  scripts over.
