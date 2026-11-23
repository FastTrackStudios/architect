+++
title = "Scripts and extensions"
description = "Let people and models add to the app without rebuilding it: Luau scripts over the Action List as one undo step, extensions with manifests, registries and hot reload, and process extensions with remote views."
weight = 130
+++

Everything earlier in this guide is built into the app. This chapter opens
it up, so a person, an organization or a model can add to it while it runs.
The one API all of it speaks is the [Action List](@/guide/commands.md): a
script, an extension and a model reach the app only through its actions.
You will:

1. run Luau scripts over the Action List, each run one undo step, and save
   them as actions of their own (also over MCP);
2. load extensions from manifests: built in, installed from a registry, or
   a development copy that reloads when its files are saved;
3. run a process extension whose view is drawn in another process and shown
   in a pane.

## 1. Scripts: `architect-scripting`

A script is a Luau program over the Action List. It lists and describes
actions, queries the read-only ones, runs the others, and loops and
branches in between:

```lua
-- Arm every selected track
local selected = state.query("TRACKS_SELECTED")
for _, id in selected do
  actions.run("TRACKS_ARM", { id = id, on = true })
  print("armed", id)
end
return #selected
```

A script's globals are `actions.list`, `actions.describe`, `actions.run`,
`actions.enabled`, `state.query` (read-only actions only), `print` / `log`,
and `args`; it returns its result.

### Install it on the list

`features/scripting` (feature `luau` for the runtime; `crdt` for undo over
a document's `History`; `settings` to keep saved scripts in a settings
file). From `features/scripting/tests/scripts.rs`:

```rust
use architect_scripting::actions::Scripting;
use architect_scripting::history::HistoryTransactions;
use architect_scripting::{Library, MemoryStore};

let scripting = Scripting::new(
    list.clone(),
    Library::new(person_store).with_org(org_store),     // where saved scripts live
)
.with_transactions(HistoryTransactions::new(doc.clone(), Arc::clone(&history)));
scripting.install()?;
```

`install` puts the `SCRIPT_*` actions on the list (`SCRIPT_RUN` runs one
once, `SCRIPT_SAVE` saves one, `SCRIPT_DESCRIBE` describes the API to a
model, plus `SCRIPT_LIST`, `SCRIPT_DELETE` and their `_ORG` forms) and
every saved script as an action of its own: "Arm every selected track"
becomes `SCRIPT_MY_ARM_EVERY_SELECTED_TRACK`, in ⌘K under "Your scripts",
bindable to a key, runnable by a model. An organization's scripts are
`SCRIPT_ORG_…`, and saving one is a separate action id so a permission
engine can keep it to some people.

### One undo step

A run opens one step (`Transactions::begin`), commits it at the end and
aborts it on failure. With `HistoryTransactions`, every `History::transact`
the script's actions make while the step is open folds into it, so the run
undoes as one. That only works if the actions and the scripting share the
app's one `Arc<Mutex<History>>` (the [documents chapter](@/guide/documents.md)).
`a_saved_script_runs_from_the_palette_and_undoes_as_one_step` is the proof.

<!-- media: script-run-from-palette-undo — a saved script run from ⌘K arming two tracks, then one ⌘Z undoing both -->

### Over MCP

Nothing extra: `SCRIPT_RUN`, `SCRIPT_SAVE` and `SCRIPT_DESCRIBE` are
actions, so a model sees them over MCP like any other. Each action a script
runs goes through the run's caller, so a model's script can do exactly what
the model may.

Limits (`Limits { memory, steps }` per run, `MAX_RUNNING` runs at once) and
a failure's line (`ScriptError { kind, line, message }`) are in
`features/scripting/src/luau.rs`. A live script hands `defer(fn)` a frame
function (ReaScript's `reaper.defer`), which the host calls one frame at a
time (`Runtime::start`, `Live::frame`).

A task board scripted from Luau, its saved scripts as actions and the board over MCP, is walked through in [Script an app, end to end](@/getting-started/scripting.md).

## 2. Extensions: `architect-extensions`

An extension is a package: an `extension.toml` manifest and its files. It
adds what the app's own features add (actions, settings sections, views,
openers, keys, overlays, document data) without a rebuild. A metronome:

```toml
id = "fts.metronome"
name = "Metronome"
version = "0.1.0"
kind = "luau"
api = "0.1"

[[actions]]
id = "FTS_METRONOME_COUNT_IN"
name = "Metronome: count in"
params = '{"type":"object","properties":{"bars":{"type":"integer","minimum":1}}}'
script = "count_in.luau"

[[sections]]
id = "metronome"
title = "Metronome"
icon = "timer"

[[sections.settings]]
key = "fts.metronome.accent"
title = "Accent the downbeat"
control = "switch"
default = true
```


- **Kinds.** `data` (settings, presets, keys; runs nothing) and `luau`
  (scripts on the scripting runtime) run built in. `process` (a program
  speaking vox, desk only) runs with `ProcessRunner`, below. `wasm` needs a
  runner the app adds; none ships yet.
- **Names are owned.** An extension's action ids start with its id's prefix
  (`fts.metronome` → `FTS_METRONOME_`), its setting keys with `<id>.`; its
  sections and actions leave with it.
- **Capabilities.** A Luau extension runs inside its `[capabilities]`
  (`actions` prefixes it may run, the most `effect` it may have); a model
  calling its action still runs as the model.

### Load them

In a Dioxus app's root component (`use_extension_sections` is
`app-shell:crates/app-shell-extensions/src/ui.rs`'s), trimmed:

```rust
let sections = use_extension_sections();                  // their settings on the settings page
let list = use_hook(|| Arc::new(ActionList::new()));
architect_commands::use_action_list(Arc::clone(&list));   // their actions in ⌘K
let extensions = use_hook(|| {
    let host = Host::new((*list).clone(), sections.clone());
    let mut ext = Extensions::new(host, Installs::new(root.join("extensions")))
        .with_source(FolderRegistry::new(registry_dir));   // or GitRegistry, ReaPackSource
    if let Some(p) = builtin() {
        ext = ext.with_builtin(p);                        // Package::embedded(manifest, files)
    }
    let _ = ext.install_actions();                        // EXT_LIST / SEARCH / INSTALL / UNINSTALL / RELOAD
    ext
});
```

- **Which copy runs** (`features/extensions/src/catalog.rs` `choose`): a
  development copy (`ext.dev(dir)`), else an installed one newer than the
  built-in, else the built-in. `disable(id)` keeps it off whatever copies
  exist; `enable(id)` turns it back on. `sync()` loads the copy of each
  that should run, reloads one whose copy changed and unloads one that has
  none.
- **Registries.** `FolderRegistry` (a folder with an `index.toml`; its
  installs are links, so an edit there reloads), `GitRegistry` (shells out
  to `git`), `ReaPackSource` (a ReaPack `index.xml`). `install(id, &VersionReq::STAR)`
  installs from the sources; `EXT_SEARCH` and `EXT_INSTALL` do the same from
  ⌘K or MCP.
- **Hot reload.** Call `reload_changed()` on a timer (every 500 ms, say). A reload is a diff: kept actions keep their ids, settings keep
  their values, and on any failure nothing is swapped and the last good copy
  keeps running.
- **A project's needs.** A document can carry the extensions it needs
  (`Requirements` in its `meta`, feature `crdt`); `Extensions::open` makes a
  `Plan` (install, offer, missing) under the org's `Policy`, and `accept`
  installs and loads it.

`features/extensions/tests/metronome.rs` installs, runs, edits and reloads
the metronome.

A registry with a Luau extension and a process extension, loaded by a host and hot-reloaded on save, is walked through in [Write an extension, end to end](@/getting-started/extension.md).

### Process extensions and remote views

A `process` extension is a program of its own. It declares its views and
actions in its manifest (`kind = "process"`, `[entry] path`,
`[[views]] id = "counter"`), and its `main` serves them
(`features/extensions/src/guest.rs` `Guest`, feature `guest`):

```rust
fn main() -> Result<(), GuestError> {
    Guest::new()
        .view("counter", Counter)                       // a Dioxus component, run in this process
        .action("LAB_CLICKER_CLICKS", |_| async {
            Ok(serde_json::json!(CLICKS.load(Ordering::Relaxed)))
        })
        .serve()
}
```

The app adds the runner to its host
(`Host::new(…).with_runner(ProcessRunner::new()?.with_processes(processes()))`,
feature `process`) and shows a view in a pane with app-shell's
`RemoteView` (`app-shell:crates/app-shell-extensions/src/remote.rs`,
feature `remote`):

```rust
View::new("clicker", "Clicker", |_| rsx! {
    app_shell_extensions::remote::RemoteView {
        processes: app_shell_extensions::processes(),
        extension: CLICKER,
        view: "counter",
    }
})
```

The host starts the program, checks its hello covers what the manifest
declares, and supervises it: when it dies the app runs on, the pane says
why with a button to start it again, and it is restarted with backoff. The
view's `VirtualDom` runs in the extension's process; its edits arrive over
vox as frames that `RemoteView` applies to a Blitz sub-document, and the
pane's keys and clicks go back. The applier trusts nothing: an edit naming
a node it does not have fails the view, not the app.
`features/extensions/tests/process_views.rs` serves a process's views.

Not yet: pointer, wheel and scroll events and IME into a remote view, stdio
as a transport, state handed over on a reload, Windows.

## Where Task and Studio are

Scripting, the extensions runtime and process extensions are
architect-side; Studio runs its features as extensions on them, drawn by
app-shell. A `wasm` runner and the WIT interface, and Task installing
extensions, are still to come.

## Where to read more

- [Extensions and scripts](@/architecture/extensions.md): the layers, the
  API as the Action List, scripts for agents, remote views, processes, hot
  reload, capabilities, registries.
- [Extensibility](@/architecture/extensibility.md): swapping a feature's
  implementation at build time (in-tree or a third-party crate), the other
  axis of extending an app.
- `skills/architect/references/commands-mcp.md` (the scripting and
  extensions sections).
