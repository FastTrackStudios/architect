+++
title = "Key bindings"
description = "architect-input's keymap engine (chords, sequences, modes, contexts, held-letter tools, MIDI and OSC, which-key), the person's keymap over the app's, and the binding editor."
weight = 42
+++

## What it is and when to use it

`architect-input` turns input into Action List ids. A `KeymapConfig` (JSON)
binds keys, sequences (`g s`), modes, mouse gestures, scroll, MIDI and OSC to
action ids, globally or in a named context (`session.arrange`) where the
innermost context wins. `InputProcessor` runs events through it and returns
`InputCommand`s; it also keeps the which-key model (what the keys typed so
far can lead to), timeouts for a pending sequence, and held-letter tools
(`Z` held while dragging). It is a pure engine: no UI, wasm-clean.

`architect-commands` layers the person's keymap over the app's: shortcuts
declared on actions are bindings, the person rebinds in the action list
window or the binding editor, and their layer syncs between devices.
Design: [Input](@/architecture/input.md) and
[Actions everywhere](@/architecture/actions-everywhere.md).

<!-- media: binding-editor-rebind — the binding editor: pressing a new chord for Close tab, a conflict warning with what the chord did, saving -->
<!-- media: which-key-panel — typing the first key of a sequence and the which-key panel listing what each next key runs -->

## A minimal example

The engine on its own, after `features/input/tests/processor.rs`:

```rust
use architect_input::{InputCommand, InputContext, InputProcessor, KeymapConfig, MacroKeys};

let mut keys = InputProcessor::from_config(KeymapConfig::from_json_str(r#"{
    "keymap":   { "normal": { "Ctrl+s": "FILE_SAVE" } },
    "contexts": { "session.arrange": { "keys": { "Ctrl+d": "ARRANGE_DUPLICATE" } } }
}"#)?)?
.with_macro_keys(MacroKeys::Off);               // not a vim app

let mut ctx = InputContext::new();
ctx.set_context("session.arrange");
for command in keys.process(event, &ctx) {
    if let InputCommand::Action(id) = command {
        let _ = list.run(&id);
    }
}
```

In a Dioxus app the commands layer does the dispatch (from
`features/commands/tests/keys.rs`):

```rust
let _ = use_commands_provider();
use_action_list(use_hook(actions));
use_keymap(
    || KeymapConfig::from_json_str(APP_KEYMAP).unwrap(),
    PersonKeymap::default,                       // or the person's kept layer
    Some(Callback::new(|person: PersonKeymap| keep_and_sync(person.to_json()))),
);
// Desk app: rsx! { WindowKeys {} } sends window chords to Commands::handle_chord
```

## Code tour

1. `features/input/src/config.rs` `KeymapConfig`: `from_json_str`, `merge`.
2. `features/input/src/processor.rs` `InputProcessor`: `from_config`,
   `process`, `sequence_timeout` and `expire_sequence`, `notify_key_release`,
   `held_tool`, `which_key`, `with_macro_keys`.
3. `features/input/src/context.rs` `InputContext`: `set_context` (dotted names,
   innermost wins), `set_tag`.
4. `features/input/src/command.rs` `InputCommand` (`Action`, `Gesture`, `Hold`,
   `Pending`) and `features/input/src/which_key.rs` `WhichKey`.
5. `features/input/src/dioxus/hook.rs` `use_input_processor`, `InputHandle`:
   the engine as a hook, callable from a listener outside the Dioxus runtime.
6. `features/commands/src/keymap.rs` `Keymap` (`bind`, `unbind`, `conflicts`,
   `what_does`, `reset`), `PersonKeymap`.
7. `features/commands/src/keys_ui.rs` `use_keymap`, `Commands::set_context`,
   `Commands::set_person_keymap`, `Commands::edit_keymap`; the components
   `BindingEditor`, `KeymapTools`, `WhichKeyPanel`.
8. `features/commands/src/ui.rs` `Commands::handle_chord` and
   `features/commands/src/native.rs` `WindowKeys`.

A keydown in the window reaches `WindowKeys`, which calls
`Commands::handle_chord("⌘W")`. That feeds an `InputProcessor` built from
`Keymap::config()` (the app's layer plus the person's) in the live
`InputContext`. It returns `InputCommand::Action(id)`, and `Commands::run`
runs the id. When the person rebinds, `edit_keymap` changes their layer and
calls `use_keymap`'s callback with the new `PersonKeymap` to keep and sync;
another device's change comes back through `set_person_keymap`.

## Tests to read

- `features/input/tests/processor.rs`: sequences, modes, counts and
  operators, which-key, release tickets, contexts, held tools, gestures, MIDI
  and OSC.
- `features/input/tests/hook.rs` (feature `dioxus`): the handle driven from
  outside the runtime.
- `features/commands/tests/keys.rs` (feature `ui`): a declared shortcut is the
  binding; rebinding; a change from another device; a sequence; a context; the
  held `Z` tool; a custom action.
- `features/commands/tests/commands.rs`: conflicts, `what_does` per context,
  resetting the person's layer.

## Status

Done in architect. Not yet: which-key's pause, keymap import and export,
importing REAPER's `reaper-kb.ini`. The sources of MIDI and OSC events are
the app's. A host that sees key releases reports them (`notify_key_release`),
or a repeated key reads as auto-repeat.

- **Task** uses the engine (`task:crates/ui/src/shortcuts.rs`, with
  `use_input_processor`); still to adopt: `architect-commands`' person keymap
  and binding editor.
- **Studio** still runs its own input crate (`studio:features/input`), where
  this engine came from; moving to `architect-input` is Studio's work.
