+++
title = "App menus"
description = "The menu bar (File, Edit, View, …) built from the Action List: native on macOS, the shell's own elsewhere, enabled and ticked as the actions are."
weight = 55
+++

> **Status: landed (2026-11-01)** in `architect-commands`: the model
> (`features/commands/src/menu.rs` `MenuBar`, `Menu`, `Standard`,
> `MenuBar::resolve`, `Tree`), ticked toggles
> (`crates/architect/src/action_list.rs` `ActionList::set_checked`),
> live sections (`Menu::section`), and four bars drawn from it: macOS
> (`features/commands/src/native_menu.rs` `NativeMenus`, AppKit through
> `muda`), Linux (`features/commands/src/linux_menu.rs`, dbusmenu on the
> session bus), iPadOS (`features/commands/src/ios_menu.rs`,
> `UIMenuBuilder`) and the in-window bar
> (`features/commands/src/menu_view.rs` `InWindowMenus`). The Linux and
> iPad bars are checked to build, not yet run on a device.

Today every app builds its menus by hand, differently: Studio's macOS
menu is AppKit code with one selector per item and no enabled or ticked
state; Session builds a File/Edit/View tree from its Action List but
serves it only on Linux (dbusmenu); Task has none. An app's menus are a
view of what it can do, and what it can do is its
[Action List](@/architecture/commands.md), so architect builds them from
it.

## What an app says

Which actions go in which menu, in what order, grouped how. The labels,
the shortcuts, whether an item is enabled, and why not, and whether a
toggle is on, come from the commands.

```rust
let menus = MenuBar::standard("Studio")              // the app menu, File, Edit, View, Window, Help, with the platform's own items
    .menu("File", |m| m.commands(["PROJECT_NEW", "PROJECT_OPEN"]).separator().commands(["PROJECT_SAVE"]))
    .menu("Edit", |m| m.group("Edit"))               // every command in the "Edit" group, by label
    .menu("View", |m| {
        m.command("VIEW_MIXER")                      // a toggle: ticked while it is on
            .separator()
            .section("Screensets", move || sets.read().menu_rows("VIEW_RECALL_SCREENSET"))
    })
    .menu("Transport", |m| m.group("Transport"));    // a menu of the app's own, before Window and Help
rsx! {
    NativeMenus { bar: menus.clone() }               // the platform's bar, where there is one
    InWindowMenus { bar: menus }                     // the app's own, where there is none
}
```

- **Ticked items:** an action that is a toggle says whether it is on
  (`ActionList::set_checked(id, || bool)`, `ActionInfo::checked`; a
  view's own command, `Command::checked`), and every bar ticks it.
- **A section is live:** `section(name, rows)` is a function the bar
  calls each time it is resolved, giving `Live` rows: a label, the
  command it runs and the arguments (JSON) it runs it with, ticked or
  not. A row whose command is not there is left out; one whose command
  cannot run is disabled with its reason. Studio's View menu of
  screensets is the line above: `Screensets::menu_rows`
  (`features/workspace/src/screenset.rs`) gives each set as a row that
  runs the app's recall action with a `Recall` of its id, the active one
  ticked.
- **When it is resolved:** the macOS, Linux and iPad bars resolve again
  when the Action Lists' events move on (`Commands::heard`: every run,
  every `list.changed(…)`, every registration) and when anything a
  section or a command reads (a signal) changes; never on a timer. The
  in-window bar resolves a menu as it opens. So an app that changes what
  a check reads outside an action says `list.changed(topic, detail)`.
- **Standard items** — Undo/Redo, Cut/Copy/Paste/Select All, Minimize,
  Zoom, Bring All to Front, Hide, Quit, Enter Full Screen — are the
  platform's, wired to the focused view (a text field gets its own
  Copy). Where the app draws or exports the bar there are none to wire
  them to, so they are left out (`without_platform`).
  An app that keeps its own history puts its Undo command in Undo's
  place (`Menu::instead(Standard::Undo, "APP_UNDO")`), so ⌘Z reaches it
  outside a text field too.

## Where it shows

| Platform | The menu bar |
|---|---|
| macOS | the native menu bar (AppKit, through `muda`): `NativeMenus` |
| Linux desktop | `NativeMenus` exports it as `com.canonical.dbusmenu` at `/MenuBar/1` (Session's `bus.rs`, moved here) and registers the window with `com.canonical.AppMenu.Registrar`; with no session bus, or no registrar to take it, the in-window bar shows |
| Windows, Linux without a global menu | `InWindowMenus`: the app's own row of menus, drawn in the shell's theme; it draws nothing while `Commands::has_global_menu` |
| iPad with a keyboard | `NativeMenus` builds it into the iPadOS menu bar with `UIMenuBuilder`: the app's rows go in at the start of UIKit's File, Edit, View, Window, Help and app menus, its own menus before Window; each row is a `UIAction` |
| Phone | none: the same actions are in the palette and each page's ⋯ |

The bar is resolved into a numbered `Tree` (menu, row, separator,
standard item; the root is 0) and each backend draws that; a pick is a
`Pick` (the command and its arguments), run on the UI's thread by
`Commands::run_pick`.

## From what exists

- Session's dbusmenu server (`studio:apps/session/ui/src/app_menu/bus.rs`)
  is the Linux backend, now fed a tree that changes (it announces each
  new one with `LayoutUpdated`) and carrying `enabled` and
  `toggle-state`. Its `app_menu::{entries, layout, Node}` and `MENUS`
  group table become the app's `MenuBar` declaration when Session moves
  to it.
- Studio's `mac_menu.rs` is replaced by the macOS backend; its items
  become actions (Audio Settings, Previous/Next Song) or the workspace's
  (screensets, panels, arrange).

## Open questions

1. **The macOS app menu's About / Services / Hide:** the platform's
   defaults, with the app adding Settings and Check for Updates?
2. **Shortcuts the menu shows but the keymap does not bind:** refuse, or
   show them greyed?
