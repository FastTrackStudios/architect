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
| `dioxus` | L4 | `dioxus::convert_*` and `dioxus::keyboard_chord` (DOM events to `InputEvent` / `KeyChord`) and `dioxus::use_input_processor`, a reactive `InputHandle` that arms the sequence timeout itself and works from outside the Dioxus runtime |

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

The processor owns no clock. After each key, `sequence_timeout()` gives
a `SequenceTimeout` ticket while a sequence is pending: arm a timer of
`ticket.after()` with whatever timer the host has, and call
`expire_sequence(ticket)` when it fires. A key that arrived in between
made the ticket stale, so a late timer expires nothing; the host keeps no
epoch counter and never cancels a timer. (`needs_timeout()` /
`timeout_expired()` remain for hosts that track that themselves.) The
`dioxus` hook does all of this.

```rust,ignore
// A browser host with no Dioxus runtime at hand (gloo-timers).
if let Some(ticket) = keys.borrow().sequence_timeout() {
    let keys = keys.clone();
    let ms = u32::try_from(ticket.after().as_millis()).unwrap_or(u32::MAX);
    gloo_timers::callback::Timeout::new(ms, move || {
        // The expired prefix comes back as `Unhandled`: dispatch it like
        // any other command.
        dispatch(&list, keys.borrow_mut().expire_sequence(ticket));
    })
    .forget();
}
```

A host that sees key releases calls `notify_key_release(&chord)` (a
release matches by key, so a keyup that lost its modifier still counts).
Without releases a second press of the same key reads as OS auto-repeat
inside a sequence, so `space space` never fires. With them, holding a
which-key prefix keeps it live across several leaf keys, and a `true`
return means "hide the overlay".

### Vim macro keys

By default an unbound `q` or `@` starts vim's macro handling: `q<reg>`
records, `q` stops, `@<reg>` / `@@` plays, and the key after them names
a register instead of running. An app that is not vim-shaped turns that
off, and `q` / `@` become ordinary keys:

```rust,ignore
use architect_input::MacroKeys;

let keys = InputProcessor::from_config(config)?.with_macro_keys(MacroKeys::Off);
```

`set_macro_keys` changes it later; `reload_config` keeps the setting.

### In Dioxus

`use_input_processor(init)` owns the processor for the component's
lifetime and returns an `InputHandle`: `handle_key` / `handle_mouse` /
`handle_wheel` / `process_event` for presses, `handle_key_release`
(a DOM keyup) or `notify_key_release(&chord)` for releases, and reactive
`current_mode()`, `pending_display()` and `is_recording()`. It arms the
sequence timeout itself.

The handle remembers the runtime and scope it was made in and re-enters
them, so it can be driven from outside the Dioxus runtime. A
capture-phase `web_sys` listener that must `preventDefault()` in the same
tick calls the handle directly:

```rust,ignore
let keys = use_input_processor(|| InputProcessor::from_config(config).unwrap());
// in the document `keydown` closure:
let commands = keys.process_event(InputEvent::Key(key), &ctx);
if claims(&commands) || keys.processor().needs_timeout() {
    e.prevent_default();
}
// in the document `keyup` closure:
let hide = keys.notify_key_release(&chord);
```

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

## Contexts

A binding can belong to a **context**: where it applies. The app says
which are live, outermost first — `ctx.set_context("session.arrange.item")`
makes `session`, `session.arrange` and `session.arrange.item` live (or
`set_contexts([...])` for names that do not nest by dots). Each context's
bindings sit in `contexts` and beat the mode's own keymap, the innermost
first; a context that binds nothing falls back outward. They are not
live in a passthrough-text mode.

```json
{
  "keymap": { "normal": { "Ctrl+d": "EDIT_DUPLICATE", "z": "VIEW_ZOOM_FIT" } },
  "contexts": {
    "session.arrange": {
      "keys": { "Ctrl+d": "ARRANGE_DUPLICATE" },
      "mouse": { "Z+Left.Drag": "VIEW_ZOOM_BOX", "Z+Alt+Left.Click": "VIEW_ZOOM_OUT" },
      "scroll": { "Z+Scroll": "VIEW_ZOOM" }
    },
    "session.arrange.item": { "mouse": { "Alt+Left.Drag": "ITEM_COPY" } }
  },
  "midi": { "CC64 ch1": "TRANSPORT_PLAY", "Note60": "MARKER_ADD" },
  "osc": { "/transport/play": "TRANSPORT_PLAY" }
}
```

## Held-letter tools and gestures

A key a pointer or wheel pattern names as held (`Z+Left.Drag`) is a
modifier while it is down. Its press comes back as `InputCommand::Hold`
(show the tool: `processor.held_tool(&ctx)` lists its gestures) and runs
nothing; gestures that follow match the patterns that hold it. The
release, `processor.release(&chord, &ctx)` (the hook's `release` /
`handle_key_up`), tells a tap from a hold by what came in between, not
by a timer: another input made it a modifier (`Unhold`); nothing did,
and it was a tap — `Unhold`, then whatever the key is bound to.

Mouse gestures are `Press`, `Release`, `Click`, `DoubleClick`, `Drag`
(`MouseEvent::drag(button, from, to, mods)`) and `LongPress`. A pointer,
wheel, MIDI or OSC match comes back as `InputCommand::Gesture { action,
event }`: the input itself, so a drag's start and delta
(`MouseEvent::delta`), a wheel's deltas or a fader's value reach the
action.

## MIDI and OSC

The binding kinds are here (`features/input/src/control.rs`). `midi`
maps `Note60`, `CC64 ch1`, `PC5` and `PB ch1` (a pitch bend); a number or
channel left out matches any. `osc` maps an address, in every mode. The
sources are the app's: it hands each message in as `InputEvent::Midi` or
`InputEvent::Osc`.

### Ports

`features/input/src/ports.rs` has the ports those messages come through,
for the app and for its extensions (#311):

- `PortMessage` is a MIDI message's bytes or an `OscMessage`, with a JSON
  form (`to_json`, `from_json`). The codecs are `midi_event` and
  `midi_bytes` (bytes to and from a `MidiEvent`), and `encode_osc` and
  `decode_osc` (OSC 1.0, bundles included).
- `PortBackend` lists ports and opens one, handing what it hears to an
  `Inbox`. `Ports` puts a MIDI backend and an OSC one together.
  `VirtualPorts` is in-process: its `VirtualDevice` is the other end, which
  a test or a demo drives. `UdpOsc` (feature `udp`, native) is OSC over
  UDP. A system MIDI backend is the app's, behind the same trait.

The types and the codecs are wasm-clean; only `UdpOsc` does IO.

### Control surfaces

`features/input/src/surface.rs` maps a device's controls to an app's
targets:

- `Target` and `Targets`: what an app lets a surface move. Each target
  kind has a range and a count, and implements read, write, label, touch
  and observe.
- `SurfaceDesc` and `ControlDesc`: faders, knobs, buttons, encoders,
  displays and LEDs, each with its input `Address`, feedback, touch sensor,
  target and strip.
- `Surface` turns input into target writes and returns the feedback to
  send. It handles banks (`set_bank`, `bank.next` and `bank.prev`
  buttons), feedback on a target's change (`changed`), touch (a touched
  fader is not moved) and `Takeover::Pickup`.

It is pure logic over `PortMessage`s; `architect-extensions` opens the
port and runs it for an extension's `control.surfaces` contribution
(`extensions.md`, "MIDI, OSC and control surfaces").

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
