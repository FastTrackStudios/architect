+++
title = "Script an app, end to end"
description = "Put actions on an Action List, drive them from Luau as one undo step, save a script as an action, and let an agent do the same over MCP."
weight = 20
+++

This walks architect's scripting API against `examples/scripting`
(`examples/scripting/src/lib.rs`, `examples/scripting/src/main.rs`, the
scripts in `examples/scripting/scripts`, and the tests in
`examples/scripting/tests/scripts.rs` and
`examples/scripting/tests/mcp.rs`). Each step points at the real file;
copy the crate to start your own.

## The mental model

```
an app's actions        ──▶  ActionList (TASKS_LIST read-only, TASKS_ADD, …)
                             + an authorizer: who may run which

Scripting::install      ──▶  SCRIPT_RUN / SCRIPT_SAVE / SCRIPT_DESCRIBE / …
                             and every saved script as SCRIPT_MY_<NAME>

a Luau script           ──▶  actions.list / describe / run, state.query,
                             each action run as whoever ran the script,
                             the whole run one step of crdt::History

architect-mcp           ──▶  the same list as tools: script_run, script_save,
                             and each saved script, for an agent
```

A script's API is the app's Action List and nothing else. Whatever a
key, a menu or ⌘K can run, a script can run — and an agent can run the
script.

## 1. The app's state

The example is a task board (`examples/scripting/src/lib.rs`, section 1):
tasks in a Loro document, with its `crdt::History`. Every change goes
through `History::transact`, so on its own each one is an undo step:

```rust
pub fn complete(&self, id: &str) -> Result<(), String> {
    let task = task_map(&self.doc, id)?;
    self.change("Complete task", |_| {
        task.insert("done", true).map_err(|e| e.to_string())
    })
}
```

`Board::transactions` hands the same document and history to scripting
as a `HistoryTransactions`. That is what makes a script run one step:
while a run's step is open, every `transact` the actions make folds into
it. Lock the history only for the change itself, never across a call
back into the Action List.

## 2. The actions

Section 2 puts four actions on the list (`register`): one read-only, the
rest changing the board.

```rust
list.register(&LIST, Effect::ReadOnly, move |f: Filter| { /* the tasks */ })?;
list.register(&ADD, Effect::Mutating, move |a: NewTask| { /* Added { id } */ })?;
list.register(&COMPLETE, Effect::Mutating, move |a: TaskId| b.complete(&a.id))?;
list.register(&REMOVE, Effect::Destructive, move |a: TaskId| b.remove(&a.id))?;
```

The argument types are facet structs, so each action has a JSON Schema a
script (`actions.describe`) and a model read. The effect matters: a
script reads read-only actions with `state.query`, which refuses
anything else. See [commands](@/architecture/commands.md) for the Action
List itself.

## 3. Who may run what

Section 3 sets the list's authorizer, a `RoleEngine`: Ana (`editor`) may
run every action; Vic (`viewer`) may run `TASKS_LIST`, `SCRIPT_RUN` and
the other read-only script actions. Services (the agent) are trusted,
and the app's own door (`ActionCaller::app()`, a key, ⌘K) is never
asked.

A script runs each action as whoever ran it, so a viewer may run a
script but the script can only do what the viewer may.

## 4. Scripting over the list

`App::new` (section 4) installs scripting:

```rust
let library = Library::new(MemoryStore::new()).with_org(MemoryStore::new());
let scripting = Scripting::new(list.clone(), library).with_transactions(board.transactions());
scripting.install()?;
```

That registers `SCRIPT_RUN`, `SCRIPT_DESCRIBE`, `SCRIPT_LIST`,
`SCRIPT_SAVE`/`SCRIPT_SAVE_ORG` and `SCRIPT_DELETE`/`SCRIPT_DELETE_ORG`,
and every script already saved. The example keeps saved scripts in
memory; a desk app keeps them in a settings file
(`architect_scripting::settings::SettingsStore`, feature `settings`).

## 5. A script that reads

`examples/scripting/scripts/inventory.luau` lists the actions it may run
(`actions.list`), describes one (`actions.describe`, with its `params`
schema as a table), and reads the board through the read-only action:

```lua
local tasks = actions.list("TASKS_")
local add = actions.describe("TASKS_ADD")
local open = state.query("TASKS_LIST", { open = true })
```

It changes nothing, so its run makes no undo step.

## 6. Branch, loop, one undo step

`examples/scripting/scripts/close_tag.luau` completes every open task
with a tag:

```lua
local tag = if args and args.tag then args.tag else "home"

local closed = 0
for _, task in state.query("TASKS_LIST", { tag = tag }) do
  if task.done then
    print("already done:", task.title)
    continue
  end
  actions.run("TASKS_COMPLETE", { id = task.id })
  print("done:", task.title)
  closed += 1
end
return closed
```

Run it:

```sh
cargo run -p example-scripting -- run examples/scripting/scripts/close_tag.luau '{"tag":"home"}'
```

The report is `{result, log, ran, undo}`: it returned 2, printed a line
per task, ran `TASKS_LIST` and `TASKS_COMPLETE` twice, and made one step,
`Script: close_tag`. One Undo takes both tasks back
(`close_tag_branches_loops_and_undoes_as_one_step` in
`examples/scripting/tests/scripts.rs`).

## 7. Errors carry their line

`examples/scripting/scripts/broken.luau` completes a task, then asks for
one that does not exist on line 9:

```sh
cargo run -p example-scripting -- run examples/scripting/scripts/broken.luau
```

The run fails with `{error: {kind: "action", line: 9, message: …}}`, and
the task it did complete is taken back with the rest of the step: the
board is as it was, with no undo step. Through `SCRIPT_RUN` the same
failure is the action's, `line 9: …`, followed by what ran and the log.
Syntax errors, Luau errors (`nil + 1`, `error(…)`) and limits
(`Limits { memory, steps }`) say their line the same way.

## 8. A saved script is an action

`Scripting::save` checks the source and keeps it; the script is then an
action of its own, `SCRIPT_MY_<NAME>` (`SCRIPT_ORG_<NAME>` for the
organization's), in the list and ⌘K, bindable to a key, taking nothing:

```sh
cargo run -p example-scripting -- save "Tidy the board" examples/scripting/scripts/tidy.luau
```

saves `examples/scripting/scripts/tidy.luau` as
`SCRIPT_MY_TIDY_THE_BOARD`, in the group "Your scripts", and runs it by
its id the way the palette does (`ActionList::run_json_async`; in an app,
architect-commands' `Commands::run`). Saving the same name again
replaces it; `SCRIPT_DELETE` takes it off the list
(`a_saved_script_is_an_action_the_palette_runs`).

## 9. An agent does the same over MCP

`App::mcp` serves the list with `architect-mcp` in named mode, so every
action is a tool: `tasks_list`, `script_describe`, `script_run`,
`script_save`, and each saved script as soon as it is saved.
`examples/scripting/tests/mcp.rs` drives it as a model would:

1. `script_describe`: the globals a script has and every action it may
   run, with schemas.
2. `script_run` with `close_tag.luau`'s source and `args` (JSON text):
   the report, one undo step named after `name`.
3. `script_run` with `broken.luau`: `isError`, `line 9: …`, nothing kept.
4. `script_save` with `tidy.luau`, then `tools/list` offers
   `script_my_tidy_the_board`, and calling it tidies the board.

Run `cargo run -p example-scripting -- mcp` to serve the board on stdio
for a real agent.

## 10. A read-only caller is refused part-way

Over MCP as Vic, `tools/list` offers no `tasks_complete`, but
`script_run` is there. Vic's run of `close_tag.luau` reads the board, then
stops at line 11, the first `actions.run`: `TASKS_COMPLETE` is not
allowed for a viewer. The failure lists what ran (`TASKS_LIST,
TASKS_COMPLETE`) and nothing changed. `inventory.luau` runs fine for Vic;
`actions.list` shows a viewer only `TASKS_LIST`
(`a_viewer_is_refused_part_way`).

## 11. A live script

`Runtime::start` runs a script's first part and keeps its Luau state;
each function it hands `defer` runs on the next `Live::frame`, each frame
an undo step of its own (ReaScript's `reaper.defer`, an immediate-mode
window's loop). `examples/scripting/scripts/burndown.luau` completes one
open task a frame until none is left:

```lua
local function tick()
  local open = state.query("TASKS_LIST", { open = true })
  if #open == 0 then return end
  actions.run("TASKS_COMPLETE", { id = open[1].id })
  defer(tick)
end
defer(tick)
```

```sh
cargo run -p example-scripting -- live examples/scripting/scripts/burndown.luau
```

A frame that fails stops the script with its line. A one-off run has no
`defer`.

## 12. Test it

```sh
cargo test -p example-scripting
```

`examples/scripting/tests/scripts.rs` runs every script in
`examples/scripting/scripts` and fails when one is added without a test.
The runtime's own tests are `features/scripting/tests/scripts.rs` and
`features/scripting/tests/live.rs`; the API in depth is
`features/scripting/src/luau.rs` (`Runtime`, `Report`) and
`features/scripting/src/actions.rs` (`Scripting`). The design is
[extensions](@/architecture/extensions.md), "Scripts and conditional
actions".
