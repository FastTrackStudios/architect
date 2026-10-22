+++
title = "Keymaps (architect-input)"
description = "architect-input: a key trie, a modal input processor and which-key, binding keys to Action List ids."
weight = 65
+++

`architect-input` (`features/input`) is the keymap engine: key chords, a
key trie, a modal input processor, mouse and scroll binding tables, a
JSON keymap config with a base + user merge, vim key notation and the
which-key popup model. It came from studio's `features/input` (#50).

A key binds to an **action id**. In an architect app that id is an
[Action List](@/architecture/commands.md) id, so a key runs exactly what
the menu, the palette, the CLI and an MCP agent run. The keymap holds no
handlers of its own.

## Features

| Feature | Layer | Adds |
|---|---|---|
| none (default) | L0 | the engine. No clock, no file IO, no runtime, no UI framework; builds for `wasm32-unknown-unknown` |
| `dioxus` | L4 | `dioxus::convert_*` (DOM events to `InputEvent`) and `dioxus::use_input_processor`, a reactive `InputHandle` that arms the sequence timeout itself |

The which-key popup is a model (`WhichKey`), not a component: each app
draws it in its own design system.

## Binding keys to Action List ids

1. Register the actions (see [commands](@/architecture/commands.md)).
2. Write the keymap. A binding is a key sequence to an action string; an
   action string is an Action List id, or `mode:<id>`, `operator:<op>`,
   `motion:<m>`, `textobj:<o>`, `unbind`.

   ```json
   {
     "modes": { "insert": { "passthrough_text": true } },
     "keymap": {
       "normal": {
         "Ctrl+s": "FILE_SAVE",
         "g g": "CURSOR_TOP",
         "i": "mode:insert"
       }
     },
     "keymap_context": {
       "normal": [{ "when": "panel:editor", "bindings": { "Ctrl+d": "EDITOR_DUPLICATE" } }]
     },
     "scroll": { "normal": { "Ctrl+Scroll": "VIEW_ZOOM" } }
   }
   ```

3. Build a processor and run what it returns:

   ```rust,ignore
   use architect::action::ActionList;
   use architect_input::{InputCommand, InputContext, InputProcessor, KeymapConfig};

   let config = KeymapConfig::from_json_str(KEYMAP_JSON)?;
   let mut keys: InputProcessor = InputProcessor::from_config(config)?;

   fn dispatch(list: &ActionList, commands: Vec<InputCommand>) {
       for command in commands {
           match command {
               InputCommand::Action(id) => {
                   let _ = list.run(&id);
               }
               InputCommand::ActionWithArgs { action, args } => {
                   // a count or an operator + motion: hand them to the
                   // action as its facet Args
               }
               InputCommand::Pending { .. } => { /* show which-key */ }
               _ => {}
           }
       }
   }

   let mut ctx = InputContext::new();
   ctx.set_tag("panel:editor");
   dispatch(&list, keys.process(event, &ctx));
   ```

The processor owns no clock. When `needs_timeout()` is true, arm a timer
of `timeout_duration()` and call `timeout_expired()` when it fires (the
`dioxus` hook does this). A host that sees key releases calls
`notify_key_release(&chord)`: holding a which-key prefix then keeps it
live across several leaf keys, and a `true` return means "hide the
overlay".

### Keep the keymap and the list in step

A key bound to an id the list does not have does nothing. Test it the
way `list.validate()` is tested:

```rust,ignore
#[test]
fn every_bound_key_names_an_action() {
    let list = app::actions();
    let keys: InputProcessor = InputProcessor::from_config(app::keymap()).unwrap();
    for trie in keys.keymaps().values() {
        for (chord_path, leaf) in trie.root().unwrap().bindings() {
            if let architect_input::LeafAction::Action(id) = leaf {
                assert!(list.info(id).is_some(), "{chord_path:?} binds unknown {id}");
            }
        }
    }
}
```

### A typed id

`InputProcessor<A = String>` takes any `A: ActionKey` (`Clone +
PartialEq + Debug + From<String>`). An app with an enum of actions
implements `From<String>`, mapping ids it does not know to an "unbound"
variant so they can be logged instead of dropped.

## User keymaps

`KeymapConfig::merge(base, user)` lays a user's file over the shipped
one. A user binding owns its whole prefix: binding `o s` drops the base's
other `o *` bindings, and binding `f` drops the base's `f f`. `unbind`
removes a sequence. The crate reads no files; the app reads the bytes
and calls `KeymapConfig::from_json_str`.

## Which-key

Hand-written keybind files often group sequences into named trees
(`WhichKeyTreeDef`: prefix `z` "Zoom", entry `t` "Toggle tracks"), in
vim notation (`<C-s>`, `gg`; `notation::translate_sequence` turns it
into keymap notation, `C-` meaning Cmd on macOS).
`which_key::bindings(&trees)` flattens them into keymap entries and
`which_key::labels(&trees)` keeps their names. While a prefix is pending,
`processor.which_key(&ctx, &labels)` lists what can follow it, from the
base keymap and every layer live in `ctx`: each entry's chord, label,
and whether it opens a group or runs an action.

## What stays in the app

Profile directories, embedded default keymaps, OS config paths, the
REAPER keybind format (`KeybindDef`, `KeybindContext`, mouse-modifier
contexts) and the keybind-editing RPC service are studio's: they name
studio's files and REAPER's sections. So are the rendered which-key
overlay and the keymap visualizer.
