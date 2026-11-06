+++
title = "Actions everywhere: bindings, ⌘K and ⌘O"
description = "REAPER's Action List, taken further: everything any UI does is an action, any action binds to any number of keys, sequences, held-letter mouse tools, MIDI or OSC, per context; which-key, a ⌘K palette and a ⌘O open-anything come free."
weight = 66
+++

> **Status: palette, quick open and the action list window landed
> (2026-11-01)** in `architect-commands` (`features/commands`): the model
> (the engine, `architect-palette`: `features/palette/src/fuzzy.rs`,
> `rank.rs` `rank`, `open.rs` `Openers`; and
> `features/commands/src/custom.rs` `CustomAction`) wasm-clean; behind `ui` the
> palette, quick open and `ActionListView` over an app's Action Lists and
> the commands a view registers (`use_command`, `use_opener`); behind
> `native` the window's chords (`WindowKeys`). The workspace's own
> commands are in `features/workspace/src/host/commands.rs`.
> **Bindings landed (#102):** keys run through architect-input
> (`features/commands/src/keys_ui.rs` `Commands::handle_chord`), with
> nested contexts, sequences and which-key (`WhichKeyPanel`), held-letter
> tools and per-target mouse gestures, MIDI/OSC bindings
> (`features/input/src/control.rs`); the person's layer over the app's
> keymap (`features/commands/src/keymap.rs` `Keymap`, `PersonKeymap`,
> kept by the app through `use_keymap`); the binding editor
> (`BindingEditor`: record a chord, conflicts shown, reset) and custom
> actions with conditions (`KeymapTools`) in `ActionListView`; the
> palette's `@`/`#`/`?` scopes (`use_palette_opener`; settings register
> `#`) and arguments asked step by step (`features/commands/src/params.rs`).
> Still to come: the `checked` test of a custom action step (with the
> Action List's `checked`), which-key's pause, keymap import/export UI and
> REAPER's `reaper-kb.ini`, scripts. Builds on the
> [Action List](@/architecture/commands.md) and
> [architect-input](@/architecture/input.md).

REAPER's Action List is the model: every single thing the program can do
is an action with an id, listed in one window, searchable, and bindable —
to several keys at once, per section (the main window, the MIDI editor),
to a mouse gesture in a context, to a MIDI or OSC message — and actions
compose into custom actions. Nothing is reachable only by clicking.

architect has the list (every action an id with its label, group,
shortcut, enabled and why not, run from keys, menus, a CLI and MCP) and
the keymap engine (sequences, modes, context layers, mouse and scroll
tables, which-key). This page is what makes it REAPER's and more, in
every app, without the app writing any of it.

## 1. Nothing is UI-only

Every button, menu row, toolbar tool, ribbon icon, context-menu entry and
gesture runs an action by id. The shell's pieces take actions, not
closures: `Tool::action("task.new")`, `Menu::action("task.delete")`,
`Ribbon` commands are ids. A component that needs a one-off handler
registers a scoped action (`use_action("song.rename", …)`) — live while
mounted, listed while it is.

So anything a person can press, they can find in ⌘K, bind to a key, pin
to the ribbon, put in a custom action, and a model can run. A check in
`architect-ui`'s tests walks a rendered app and fails on a pressable
element with no action.

A view's own command (`architect_commands::Command`, registered with
`use_command`) says what running it does, as an action's `Effect` does:
read-only, mutating (the default) or destructive (`Command::destructive`,
`Command::read_only`; `features/palette/src/item.rs` `Effect`). A model
reaches commands through `COMMANDS_RUN`, and runs each one with that
command's own effect, not as a blanket write
(`features/commands/src/bridge.rs` `use_command_actions`):
- a destructive command waits for a person's yes, as a destructive
  action does (the Action List's confirmation tickets);
- an action run through `COMMANDS_RUN` is authorised as itself;
- `COMMANDS_LIST` lists every command with its effect.
- `COMMANDS_SHOW` opens the palette or ⌘O with its field set (`theme:` lists one opener alone).

## 2. Contexts

A binding belongs to a **context**: where it applies. Contexts nest —
the app, then a workspace view (`session.arrange`), then a target under
the pointer (`session.arrange.item`, `.ruler`, `.envelope`) — and the
most specific one that binds a key wins, falling back outward (REAPER's
sections, made a tree). A view declares its contexts and the targets in
it (what the pointer is over), and the workspace tells the keymap which
are live: the focused pane's view, and the target under the pointer.

## 3. Bindings: many, of every kind

An action has any number of bindings, in any contexts, of any kind:

| Kind | Example |
|---|---|
| Key chord | `⌘S` |
| Sequence (with which-key) | `g g`, `<Space> f o` |
| Held key + mouse | hold `Z` + drag → zoom; hold `Z` + click → zoom to; hold `Alt+Z` + drag → zoom out |
| Modifier + mouse, per target | `⌥ + drag` on an item → copy item (REAPER's mouse modifiers) |
| Wheel / trackpad | `⌘ + scroll` → zoom; pinch → zoom |
| MIDI | CC 64 on channel 1 → play; a note → a marker (control surfaces) |
| OSC, gamepad | `/transport/play` |

**Held-letter modifiers** (quasimodes) are new: any key can act as a
modifier while held, so a letter is both a tool and a key. Hold `Z` and
the pointer becomes the zoom tool — drag a box, click, scroll — and its
cursor and the which-key hint say so; let go, the tool goes. A tap of `Z`
on its own still runs whatever `Z` is bound to. The keymap tells the two
apart REAPER's way, by what arrives next, not by a timer: a key that is
down when another input comes (a pointer gesture, another key, the wheel)
is a modifier for that input; a key that goes up with nothing in between
was a tap. The same rule holds for every key, the platform modifiers
included. Studio's `W` (arrange the workspace) is the same mechanism.

A mouse binding is `[held keys +] [modifiers +] button.gesture @ target`:
gestures are click, double-click, drag (with its start and its delta),
long-press, and the wheel. A drag binding gets the gesture's data (from,
to, delta, the target under its start) as the action's arguments.

## 4. The action list window (REAPER's)

One window, every app: every action, searchable by name, id, group and
binding; for each, all its bindings by context, with add (press the
chord, click the gesture, send the MIDI), remove, and conflicts shown
("⌘D is also Duplicate in Arrange"). Filter to a context; show only
unbound; show what a key does in each context. Run an action from the
list; copy its id. Import and export a keymap; reset one action, a
context, or everything, to the app's defaults. It is a [settings
section](@/architecture/settings-page.md) ("Keyboard & mouse") and a
workspace view.

## 5. Custom actions and scripts

A custom action is a list of actions run in order (REAPER's), with their
arguments, named by the person, and itself an action: bindable, in ⌘K,
pinnable, runnable by a model. Made in the action list window by picking
actions; stored with the keymap.

Custom actions can be **conditional** (SWS's): a step runs if a test
holds (an action's `checked`, a value from a query action, the context),
and steps can repeat. Past what the list editor can say, a **script**
is an action too: a small program that queries the app through its
read-only actions and runs others, with conditions and loops (REAPER's
ReaScript). Scripts and the extensions that add actions, views, settings
and openers at runtime are one design, settled separately
([extensions](@/architecture/extensibility.md); the runtime half is
proposed). Scripts have landed: `architect-scripting`
(`features/scripting/src/actions.rs` `Scripting`) runs Luau over the
Action List as one undo step, and keeps saved scripts — the person's and
the organization's — as actions of their own (`SCRIPT_MY_…`,
`SCRIPT_ORG_…`).

## 6. Which-key

While a sequence is pending, or a held-letter mode is down, a panel shows
what can follow: each next key with its label, groups marked as groups
(the `WhichKey` model, drawn by the shell). It shows after a short pause
(not for someone who knows the keys), and lists the mouse gestures the
held key enables ("Z: drag — zoom to box · click — zoom in · ⌥click —
zoom out").

## 7. ⌘K: every command

A palette over everything (the shell draws it; [app shell](@/architecture/app-shell.md)):

- **What it lists:** every enabled action in the live contexts, the most
  specific first; disabled ones dimmed with why; recent first when the
  field is empty. Each row: the action's name, its group, its bindings.
- **Arguments:** an action that takes some asks for them in the palette,
  a step at a time, from its params schema (a choice, a number, text) —
  "Recall screenset ▸ Mix".
- **Prefixes** narrow it: `>` commands (the default), `@` this page's
  targets (a track, a section), `#` settings ("# buffer" opens the row),
  `?` help for a key.
- Opened by ⌘K, the shell's pull-down on a phone, and an action.

## 8. ⌘O: open anything

What can be opened is a registry of **openers**: each feature registers
one — Studio's projects, setlists, songs, sessions, rigs; Task's notes,
tasks, projects; any app's workspace views and screensets; files. An
opener answers a query with items (a title, a detail, an icon, how
recent) and opens one by an action (`workspace.open` with where: here, a
new tab, a split, a window — ⌘↵ for a new tab, as in Obsidian).

⌘O searches every opener at once, recent first when empty, grouped by
kind; a prefix narrows to one (`song:`). Studio's library finder and
Task's quick switcher become openers.

<!-- doc-check: off (proposed names) -->
```rust
Openers::register(Opener::new("song", "Songs", icon!(Music), |q| songs.search(q), |id| Open::view("session.song").state(id)));
```
<!-- doc-check: on -->

## 9. Where bindings live

Each app ships a keymap per platform (its defaults, by context). A
person's changes are a layer over it ([user keymaps](@/architecture/input.md)),
kept with their settings (synced), so their keys follow them. A keymap
can also be imported from REAPER's `reaper-kb.ini` where an app maps its
actions to REAPER's (Studio).

## From what exists

| Exists | Grows |
|---|---|
| `ActionList` (`ActionInfo`: id, label, group, shortcut, enabled) | `checked`; contexts an action belongs to; scoped actions (`use_action`) |
| architect-input: chords, sequences, modes, `when` layers, mouse and scroll tables, which-key | held-key modifiers; per-target mouse gestures with gesture data; MIDI/OSC sources; nested contexts |
| Studio: REAPER keybind format, mouse-modifier contexts, the keybind editing service | the action list window, generic; REAPER import |
| Studio's library ⌘K / ⌘O, Task's quick switcher | the shell's palette and the openers registry |

## Open questions

1. **MIDI/OSC bindings** in architect, or Studio's (control surfaces are
   Studio's world)? Proposed: the binding kinds in architect, the sources
   plugged in by the app.
