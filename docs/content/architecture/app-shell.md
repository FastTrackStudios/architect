+++
title = "The app shell"
description = "One shell for every FastTrackStudio app: a title and its actions at the top, four tabs and a separate button at the bottom, a sidebar on each side — laid out for a phone, a tablet and a desk from one declaration."
weight = 53
+++

> **Status: first cut landed (2026-10-31)** in `architect-shell`
> (`features/shell`), with a sandbox to develop it in (`apps/sandbox`, the
> Architect app on made-up data; it and the iOS app run under `dx serve`). Task's shell is the
> reference; Studio is the second app on it.

Task has a shell people like: on a phone, the screen's name large at the
top left and its main action at the top right, four tabs in a floating
pill at the bottom with one more button set apart beside it, and a
drawer on each side a swipe away. Studio wants the same shell
(`studio:docs/design/studio-app-shell.md` already says "modelled on
Task's shell"), and so will the next app. This page moves it into
architect: one shell, a few styles, and every app declaring what goes in
it rather than drawing its own chrome.

What exists today:

- **Task** draws it itself: `task:crates/ui/src/shell/app_shell.rs`
  (the layout), `task:crates/ui/src/shell/mobile.rs` (`BottomTabBar`,
  the capture button), `task:crates/ui/src/shell/phone_gestures.rs`
  (the drawers), `task:crates/ui/src/shell/rail.rs`,
  `task:crates/ui-core/src/section_header.rs` (`SectionHeader`, the top
  bar, on 17 screens) and `task:crates/ui/src/nav_layout.rs` (the
  person's order of the tabs).
- **Studio** draws its own by hand (`studio:features/library/src/ui/mod.rs`:
  an app bar, a sidebar or rail, a tab bar on a phone), with its own
  rule for what size is a phone (`studio:features/app/shell/src/ui/form.rs`,
  `FormFactor`).
- **architect-ui** has the parts but no shell:
  `features/ui/architect-ui/src/components/sidebar.rs`,
  `features/ui/architect-ui/src/components/side_sheet.rs`,
  `features/ui/architect-ui/src/components/drawer.rs`,
  `features/ui/architect-ui/src/components/nav_tab.rs`.

## The parts

Six slots. An app fills them; the shell decides where each one goes.

| Slot | What it is | Who fills it |
|---|---|---|
| **Title** | The screen's name, large, at the top left (or something to lead with instead: Task's Schedule leads with its week strip) | the page |
| **Main action** | One action, at the top right, in a pill with the ⋯ | the page |
| **⋯ menu** | Everything else the screen offers: filter, display, what is particular to it. A badge on the ⋯ counts what narrows the view, so a filtered screen is never a mystery | the page |
| **Tabs** | The app's sections. The first four are the bar | the app; the person orders them |
| **The separate button** | One button set apart from the tabs: Task's capture, Studio's search and open | the app |
| **Left sidebar** | The screen's companion column: where you are and what is beside it (a project's tree, the vault's files, a library's folders) | the page, else the app's default |
| **Right sidebar** | What the screen is about: the selection's properties, links, comments, a docked panel | the page, else the app's default |

The title, action and ⋯ belong to the screen, not the app: a page states
them once and the shell draws them wherever this form factor puts them.
A page never builds its own top bar.

## Laid out by form factor

Phone, tablet and desk are **sizes, not devices**: a desk window sized
like a phone is laid out as a phone. Touch is an input style beside the
size (larger targets, no hover), never what picks the layout. The rule
moves here from Studio, so every app switches at the same sizes:

- **Phone**: upright and narrower than 700pt, or on its side and
  shorter than 500pt.
- **Tablet**: anything larger, under a finger.
- **Desk**: anything larger, under a mouse.

| | Phone upright | Phone on its side | Tablet | Desk |
|---|---|---|---|---|
| Title, action, ⋯ | top of the screen | top of the screen | top of the content column | top of the content column |
| Tabs | a floating pill along the bottom | a rail down the left side | the left sidebar's head, folding to a rail when upright | the left sidebar's head |
| Separate button | beside the pill, bottom right | at the rail's foot | floating, bottom right | in the top bar, right of the ⋯ |
| Left sidebar | a drawer: swipe right | a drawer: swipe right | docked; folds to the rail when upright | docked, resizable |
| Right sidebar | a drawer: swipe left | a drawer: swipe left | over the page below 1100pt, docked above | docked, resizable |

Sideways a phone's height is what is short, so a bar along the bottom
would take the room the content needs: the tabs run down the side.

### The top bar

Task's `SectionHeader`, as it is: the title at 28pt bold on the left;
on the right one pill holding the main action (an icon, optionally with
its words: "Process 5") and the ⋯, split by a hairline. The ⋯ opens a
menu of rows 48pt tall (an icon, a label, an optional note: "2 on"),
grouped by hairlines; picking a row closes it. On a tablet or desk the
same items are also commands in the app's command palette, so nothing
in the ⋯ is mouse-only.

On an upright phone the bar's buttons say their icons alone, whatever
the page asked for (their words stay their names): the title, or the
app's brand on a root page, has the rest of the row, and a title too
long for it is cut at the buttons. Keys shown beside what they do
(`⌘O`, "↵ open") are a desk's: a page asks `Shell::shows_keys` before
drawing one, which is false on a touch screen.

### The bottom bar

Task's `BottomTabBar`: a pill 54pt tall floating 12pt above the
bottom inset, holding four icon-only tabs (the current one filled, a
badge where a tab has something waiting); beside it, a round 54pt
button with the separate action. The page pads its foot by the bar's
height so the last row is never under it. A page's own sticky actions
(Task's `MobileActionBar`) sit just above the pill.

### The action bar and the keyboard bar

Over the tabs on a phone sits the action bar: up to six (`BAR_MAX`)
icon buttons, each an Action List command by id with a badge if it
counts something (Obsidian's iOS bottom bar). The app gives its own
(`AppShell`'s `actions`); a page adds its own while it is mounted
(`use_bar_action`), in place of the app's of the same id, and past six
the app's give way from the end. The workspace's tabs button is one
such: `features/workspace/src/bar.rs` `use_tabs_button` counts the
main window's tabs and runs `WORKSPACE_EVERY_PANE`, the phone's tab
switcher. A
press runs the command through the `CommandRunner` in context, which
`architect-commands`' `use_commands_provider` puts there, so a button
does what the palette and the keys do and is dimmed when the command
cannot run.

A page can also take the whole screen, the bars too, while it is
mounted: `features/shell/src/shell/mod.rs` `use_shell_cover` draws its
element over everything in the shell's own box. The workspace's tab
switcher is one (`features/workspace/src/host/switcher.rs`), as
Obsidian's covers the screen on iOS.

While a field is typed in, the bar rises above the keyboard and becomes
the **Typing Toolbar** (Task's `KeyboardToolbar`, `task:crates/ui/src/doc_editor.rs`;
`features/shell/src/shell/typing.rs`): its items in a pill that scrolls
sideways when they do not fit, and apart at the right a round button that
puts the keyboard away, which is the shell's and never an item. When the
field lets go, the bar settles back into the action bar and the tabs. The
app writes none of it: `features/shell/src/shell/bar.rs` `PhoneFoot` lays
it out.

Its items come from three places:

- **The focused editor's own.** The editing actions it registers
  (`use_editing_actions`, with a context such as `markdown`) are the
  built-in items, so an app with no extensions shows what it always did.
- **Toolbars of place `typing`.** Whoever puts `features/shell/src/shell/typing.rs`
  `TypingTools` in context fills the bar, given the `Typing` in force: the
  editor's context and actions, and the pages and views the focus is in.
  A page or view says it may hold the focus with `use_typing_scope`
  (`view:<kind>`), wired to its box's `onfocusin`/`onfocusout`; every
  workspace pane does (`features/workspace/src/host/group.rs` `PaneSlot`).
  `architect-extensions` fills it (`features/extensions/src/typing_ui.rs`):
  the editor's items as the built-in toolbar of its context
  (`typing/<context>`, `features/extensions/src/toolbars.rs`
  `Toolbars::provide_editor`), then every extension's `[[toolbars]] where =
  "typing"` whose `when` is in force (`when = "view:lab.board/chart"`,
  `editor:markdown`), from its manifest or added at run time while its page
  is mounted (`linked::Handle::contribute`, `ext.contribute`). An
  extension's items show only while its page or view has the focus.
- **A person's edits.** The built-in toolbar of a context is a toolbar
  like any other: the toolbar editor (`docs/content/architecture/toolbars.md`)
  reorders, hides or adds items, and its copy is written as any toolbar
  file is (`toolbars/typing-<context>.toml`), used in place of the
  editor's own until it is reset. With the editor on, the bar's toolbars
  show above it, edited in place.

Items are buttons (an icon, or a name: `♯`, `C`), toggles that show on
while their action says so (`aria-pressed`, from the action's `checked`
state: an extension's `toggle = true` action, or a command's `checked`
through `CommandRunner::checked`), dropdowns whose rows open **above** the
bar, separators, and widgets (an extension's view as a live control). A
group's buttons sit in the row one after another. The shell draws them
all, so they look alike whoever put them there: `TypingItem` (`Button`,
`Separator`, `Menu`, `Widget`), each a width the pill scrolls by.

Pressing any of them keeps the field's focus, and so the keyboard: the
pill, a dropdown's rows and a widget's box prevent the default of a
`pointerdown`, a `mousedown` and a `click`
(`skills/architect/references/blitz-quirks.md`). The same bar is drawn
by Blitz on iOS and Android and in a desktop window sized like a phone,
and by a web view or a phone's browser, which tell the shell the
keyboard from `visualViewport` and `focusin`.

The architect-shell crate does not depend on the extensions: it exposes
the slot, and `architect-extensions` (which already depends on the
shell) fills it, as the workspace's `HeaderTools` is filled for a pane's
header.

The keyboard is one process-wide feed (`features/shell/src/shell/keyboard.rs`):
its height comes from UIKit's keyboard notifications on iOS
(`install_keyboard`, before launch) and from `visualViewport` in a
browser or a web view; whether a field has the focus comes from
`focusin` / `focusout` there, and in Blitz from the document's text
focus, which the host forwards
(`dioxus_native::on_text_focus_change(architect_shell::set_typing)`).
The bar eases after the keyboard on the shell's `Motion`. On iOS the
same call takes a web view's own ⌃ ⌄ ✓ accessory bar away (moved here
from `task:apps/mobile/src/keyboard_bar.rs`).

Where the keyboard is the screen's own (iOS: `soft_keyboard`), it comes
up only when the person asks for it, as in any iOS app:

- **A tap that ends on a field.** The shell's root follows each finger
  (`features/shell/src/shell/keyboard.rs` `TapStroke`: past `TAP_SLOP` it
  is a scroll), and its click brings the keyboard up when a field has the
  focus (`tap_raises`). A press that becomes a scroll, and an `autofocus`
  (a note opening), give the field the focus and no keyboard: Blitz asks
  winit's view to become the first responder whenever a field is focused,
  and `features/shell/src/shell/keyboard/ios.rs` refuses it unless a tap
  or a button opened the gate a moment before.
- **The keyboard button.** A field with the focus and the keyboard down
  (after Hide, an `autofocus`) shows a keyboard button first on the
  action bar (`Keyboard::foot`, `Foot::TabsWithKeyboardButton`).
- **An app that opens a field on purpose** calls `show_keyboard` (the
  capture composer does, from its button).

On such a screen the editing bar rides the keyboard only while it is up;
`hide_keyboard` (the bar's round button) resigns the keyboard whatever
holds the focus. Elsewhere (a desk window standing in for a phone) a
typed field has the editing bar, as before.

`ARCHITECT_INPUT_DEBUG=1`, or the Architect app's ⌘K command "Debug:
touches and keyboard" (`SHELL_INPUT_DEBUG`), draws what the shell hears
over the app (`features/shell/src/shell/input_debug.rs`): a cyan line at
the keyboard's top as UIKit reported it and a magenta one at the foot's
bottom edge as laid out (`foot_bottom`), the keyboard and the screen's
insets, whether winit's view has the keyboard, and the last presses with
what in the foot heard them. On a phone the two lines meet while the
keyboard is up; where they do not, the reported height is wrong.

The person orders the tabs (Task's Customize: drag to move, hide, bring
back), per device. The first four are the bar; the rest are in the left
sidebar's list and the command palette. A section added later (a plugin
switched on) appears at the end until it is moved.

### The sidebars

On a phone each side is a drawer, opened by a swipe from anywhere on the
screen, closed by a swipe back, a tap on the dimmed page, or going to
another screen. A pull down from the top bar opens the command palette.
A stroke is never taken from something that needs the drag itself (a
horizontally scrolling strip, a slider, a dialog, anything marked
`data-no-swipe`, a long press). On a tablet and a desk the drawers are
sidebars, and nothing listens for the strokes. As a phone's drawer comes
in, the page gives way by about a fifth of the drawer's width, moved by `left`
(Blitz moves a `translate` by device pixels; see `blitz-quirks.md`).

A page that fills a side claims it while it is mounted, and the app's
default for that side steps aside: the content is rendered inside the
page's own tree, so its signals are read under the scope that owns them
(Task's `PhoneDrawer`). The shell holds which side is open, the widths
(persisted per device) and whether a side is folded.

## Styles

One shell, a few looks an app chooses:

| Style | What changes |
|---|---|
| `FloatingPill` (the default, Task's) | The tabs in a floating pill, the separate button round beside it, the ⋯ pill at the top |
| `TabBar` | The iOS tab bar: full width, labels under the icons, the separate button as a fifth item set apart at the right end |
| `RailOnly` | No bottom bar on any size: a rail on the left with the separate button at its foot. For apps used mostly at a desk |

A style changes how the slots look, never which slots there are, so an
app can change style without touching a page.

## The API

A sketch, to be settled in review:

<!-- doc-check: off (proposed names; nothing here exists yet) -->
```rust
// The root, once per app. The app's Route stays the app's: the shell is
// told which tab is current and what a press on one does.
rsx! {
    AppShell {
        style: ShellStyle::FloatingPill,
        tabs: vec![
            Tab::new("home", "Home", rsx! { House {} }),
            Tab::new("sessions", "Sessions", rsx! { AudioWaveform {} }),
            Tab::new("signal", "Signal", rsx! { Guitar {} }),
            Tab::new("charts", "Charts", rsx! { Music {} }),
        ],
        current: current_tab(),
        on_tab: move |id| go_to(id),
        separate: Separate::new("Search and open", rsx! { Search {} }, move |()| palette.open()),
        left: rsx! { LibraryFolders {} },     // the app's default left side
        right: None,                          // nothing by default
        Outlet::<Route> {}
    }
}

// A page states its top bar; the shell draws it where this size puts it.
rsx! {
    PageBar {
        title: "Sessions",
        action: Action::new("New session", rsx! { Plus {} }, move |()| new_session()),
        badge: filters_on,
        MenuItem { label: "Filter", icon: rsx! { Funnel {} }, on_click: move |()| filter.open() }
        MenuItem { label: "Display", icon: rsx! { SlidersHorizontal {} }, on_click: move |()| display.open() }
    }
    // Claims the right side while this page is mounted.
    RightSide { SessionInspector { id } }
    SessionList {}
}

// What the shell is laid out for, from the app's host.
let screen: Screen = use_screen(); // size, form factor, touch, safe-area insets
```
<!-- doc-check: on -->

It is `architect-shell` (`features/shell`,
[one crate per feature](@/architecture/layout.md)): `Screen` and the
form-factor rule in the default (wasm-clean) build, and behind `ui` the
rest — `AppShell` (`features/shell/src/shell/mod.rs`), the pages and
their bars (`features/shell/src/shell/page.rs` `Page`, `Toolbar`,
`Breadcrumb`, `Menu`), the bottom bars, the drawers and their gestures,
and the workspace switcher (`features/shell/src/shell/workspaces.rs`).
The names above are the sketch; the code is what landed.

## On a desk: Obsidian's frame

A desk window is laid out the way Obsidian lays out a vault, and every
app on the shell gets it:

- **The title bar** runs the whole width, 40pt
  (`WindowChrome::MAC_ROW`), in its own colour (`ShellTheme::title_bar`)
  over one rule. The traffic lights are placed in it
  (`fit_traffic_lights`, after each resize), then the sidebar's button.
  What fills the rest is **tabs**: a page's own tab (a `Page`'s title),
  or a workspace's tab bars. A page whose top edge is its tabs claims
  the row (`features/shell/src/shell/mod.rs` `use_title_row`;
  `features/workspace/src/host/mod.rs` `WorkspaceView` does when given a
  `TitleRow`): the page, and a left sidebar it fills, are laid out from
  the window's top, so the panes' tab bars along the top edge are the
  title row, one row and never two; the shell then draws only its
  pieces of the row over the ribbon, the app's sidebar and the right
  side beside them (not one row under the page: Blitz finds a press only
  inside its element's parents' boxes). The leftmost
  bar keeps clear of the traffic lights and the sidebar's button
  (`Shell::title_clearance`). The tab on show is the page's colour and
  runs on into it. The breadcrumb and the page's toolbar sit in the
  page's header under it, where Obsidian shows a note's path. A press on
  the bar's empty part moves the window (the tab bars' own empty part
  calls `WindowChrome::on_drag`). Linux and Windows draw the same in-app
  row (52pt without a `WindowChrome`); on a Mac the traffic lights are
  AppKit's, put in the row by `features/shell/src/shell/mac.rs`
  `fit_traffic_lights`, which the sandbox calls from
  `apps/ui/src/screen.rs` `use_window_chrome` after each resize.
  A pane popped out to a window of its own gets the same row: on a Mac
  its launcher makes the title bar transparent, puts the traffic lights
  in it and hands the pop-out a `TitleRow`
  (`features/workspace/src/native.rs` `popout_title_row`), so its tab
  bar is the title row, clear of the lights, its empty part moving the
  window.
- **The ribbon** (`RIBBON`, 44pt) is the window's left edge under the
  bar, the full height: the app's sections as icons, then what the page
  adds (a workspace's views and pinned commands). Folding the sidebar
  leaves the ribbon.
- **One sidebar** beside it: the app's `left`, or what the page claims
  with `use_shell_slots` (a workspace's left dock, its tabs in the title
  row). The app's foot buttons (Theme, Settings), the workspace
  switcher and the account stay in the ribbon's foot as their marks
  (the workspace's initial, the person's avatar), the sidebar open or
  not: they never move into it.
  The left side has three layouts (`SidebarLayout`): the rail alone,
  a sidebar of one column, or of two (Notebook Navigator's single and
  dual pane). Every page can be shown in any of them (the sidebar's
  button, ⌘K's "Sidebar: …" from `architect_commands`'
  `use_sidebar_commands`); a page says which it prefers
  (`use_sidebar_layout`, or `Page`'s `sidebar`); a page that says
  nothing gets one column when there is something in it and the window
  is 1000pt wide, the rail otherwise. The person's own choice wins and
  is kept per page kind (the current tab). Two columns start 480pt
  wide; back to one, the sidebar has one column's width again. A
  sidebar has no maximum width: dragged wider, it stops where the page
  would have less than 240pt.
  Phones and tablets keep their own drawers and rails.
- **The status line**: `StatusItem`s (a fact or a switch: text, icon, a
  dot, a chip, a press), the app's on `AppShell` and the page's from
  `use_status`. It is drawn as a `StatusStyle::Bar` along the foot,
  right of the ribbon (Task's: the page's items on the left, the app's
  on the right), or as a `StatusStyle::Badge` in the bottom right corner
  (Obsidian's).

## It renders in Blitz

Studio's windows, its plugins and its iPhone app paint through Blitz
(Vello + wgpu), not a web view, so the shell must render the same there
as in a browser. What Task's shell does today that Blitz does not:

| Task does | Blitz has | The shell does |
|---|---|---|
| `position: fixed` for the bars and drawers | no `fixed`, no `sticky` | positions against the shell's root, which fills the window (`inset: 0`) |
| `env(safe-area-inset-*)` | no `env()` | takes the insets from the host through `Screen` (Studio's iOS scene already publishes them) |
| `backdrop-filter: blur` on the pills | no backdrop filters | an opaque fill of the same colour; the blur is added where the renderer has it |
| a Tailwind stylesheet loaded by the page | no external stylesheets | the layout is inline styles; colours and type come from the theme tokens, and a class only adds to what is already laid out |
| `placeholder`, `text-overflow: ellipsis` | neither | a hint drawn under an empty field; text clipped by `overflow: hidden` |
| a document-level script for the swipes | no scripts, but `ontouchstart` / `ontouchmove` / `ontouchend` reach Dioxus handlers on iOS (tried 2026-10-30, below) | in Blitz, touch handlers on the shell's root; on the web, the script as now. Rust classifies the strokes the same way in both |

### Touch in Blitz on iOS, tried

On 2026-10-30, Studio's iOS build (Blitz `fts-pointer-2026-10`) ran on
an iPad simulator with touch handlers on the library's root and a
scrolling list inside it, driven by real simulated touches (`idb ui
swipe`, which goes through the simulator's HID, as a finger does):

| Stroke | What the root's handlers saw | The list |
|---|---|---|
| Up the list, 300pt | start, 31 moves, end; no cancel | scrolled 13 rows |
| Right across the list, 420pt | start, moves, end: dx 420, dy 6 | moved about 4pt |
| Left across the list, 420pt | dx −420, dy −4 | moved about 4pt |
| Down from the top bar, 355pt | dx 5, dy 355 | — |

So the drawers' strokes work in Blitz as they are: Blitz turns a finger
into pointer events and `touch*` events (`blitz-dom`'s event driver),
and dioxus-native-dom hands those to Dioxus handlers. Unlike iOS's web
view, **a scroll does not cancel the stroke**: the scroller and the
handlers both get all of it. That makes recognition simpler (no
cancelled strokes), and it means the shell must not act while the
stroke is undecided: it classifies after 12pt, as Task's does, and a
sideways swipe still nudges a list under it by its small vertical part.
Whether a handler can stop the scroll once it has claimed a stroke
(`prevent_default` on `touchmove`) was not tried; the shell does not
need it to work.

A tap on a text field brings the keyboard up, so a pull-down only
counts when it starts on the bar itself, not on a field in it.

Each scene is checked at the three sizes in `features/ui/ui-snapshot`
(`features/ui/ui-snapshot/src/scenes.rs`) with pixel probes, the way the
components are today.

## Moving the apps onto it

- **Task first**, as the reference: its shell, `SectionHeader`,
  `BottomTabBar`, the drawers and the tab order move onto architect's,
  one PR per piece, changing nothing a person sees. Task's tabs stay
  Inbox, Tasks, Schedule, Vault, and its separate button stays capture
  (swipe up for a timer).
- **Studio next**: Home, Sessions, Signal and Charts as the tabs, search
  and open as the separate button. Each view's main action is its own
  (New session in Sessions, Load rig in Signal, New chart in Charts),
  with the rest under the ⋯. The library's own app bar, sidebar and tab
  bar go; `studio:docs/design/studio-app-shell.md` points here for the
  shell and keeps only what is Studio's.
- Studio, Task and keyflow pin the same architect tag, so the shell
  lands in all three at one bump.

## Not the shell's

- **Routing.** The shell is told the current tab and calls back on a
  press; each app keeps its own `Route`
  ([composing the UI](@/architecture/composing-the-ui.md)).
- **What is in the sidebars.** The shell holds them open, folded and
  sized; the app and its pages fill them.
- **The account, org switcher and presence.** They go where the app
  puts them (Task: the left sidebar's foot and the More sheet).

## Open questions

1. **Stopping the scroll under a claimed swipe.** Touch works in Blitz
   (above); can a `touchmove` handler that has claimed a sideways
   stroke stop the list under it from also moving? Not needed for a
   first version.
2. **The separate button's second gesture.** Task's swipes up for a
   timer. Is a second gesture part of the slot (`Separate::on_swipe_up`),
   or Task's own?
3. **A More tab.** Task's doc mentions a fifth "More" tab drawn as the
   avatar; the bar ships four. Does the account move to the left
   sidebar's foot everywhere, or does a style keep a More tab?
4. **The right sidebar on an iPad on its side.** Docked from 1100pt
   leaves the page 600pt or so between two sidebars. Overlay until
   1280pt instead?
