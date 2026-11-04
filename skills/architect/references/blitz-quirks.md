# Blitz quirks found in architect's shell and lab

Architect's shell, workspace host and lab paint through Blitz (`dioxus-native`:
Stylo, Taffy, Parley, Vello), not a browser. The general rules (what CSS and
which elements Blitz supports) are in Studio's `blitz-design` skill
(`studio:.claude/skills/blitz-design/SKILL.md`). This page lists what building
the shell and the lab found on top of that, each with what to write instead.
They belong in that skill too; until Studio takes them, they are kept here.

## Layout and text

| What happens | Write instead | Where architect does |
|---|---|---|
| In a dock, a flex child's wrapping text measures at its narrowest (min-content), so a centred empty state wraps a word per line. | Lay the block against the pane's own edges: `position: absolute; top: 0; left: 0; right: 0; bottom: 0`, then flex inside it (and see the collapsing pane below). | `apps/lab/ui/src/vault.rs` `Stand` |
| A space where two spans meet is dropped, even a non-breaking one (`&nbsp;`, `\u{a0}`): "a **b** c" reads "a**b**c". | Trim each run and turn its edge spaces into margin on `display: inline-block` runs, about 0.27em a space. | `features/commands/src/ui.rs` `Lit` |
| `align-items: baseline` in a flex row lines an item up with a wrapping neighbour's last line, not its first (a browser uses the first): a label beside two lines of text sits low. | `align-items: flex-start` when a neighbour can wrap, and the same font size or a `padding-top` to line the first lines up. | `apps/lab/ui/src/pages.rs` `HelpPage` |
| The `ch` unit is not supported. | Size in `px` or `em`. | |
| `placeholder` on an input draws nothing. | Put the question in a `label` above the field. | `apps/lab/ui/src/pages.rs` `CapturePage` |
| An element measured in `onmounted` can report a zero rect: it is not laid out yet, and nothing measures it again. | Measure again when the number is needed (a menu opening, a drag starting), and ignore a zero size. | `features/workspace/src/host/mod.rs` `Host::open_menu` |
| A `title` attribute draws no tooltip. | Draw the tip: a child `span.tip` in a `position: relative` host, shown by `.tip-host:hover > .tip` (Blitz marks the hovered node's ancestors hovered, as a browser does) after a transition delay. | `features/shell/src/shell/mod.rs` `shell_css`, `RibbonTab` |
| `transform: translate…` moves by device pixels, not CSS pixels: on a 3x phone a `translateX(-90px)` moves 30 points (hit-testing undoes the transform at the right scale). | Move a box with `left` / `top`: on an absolutely placed element, or on a relatively placed one, which keeps its place in the flex box. | `features/shell/src/shell/bar.rs` `KeyboardBar` (the pill's scroll); `features/shell/src/shell/mod.rs` `AppShell` (the page giving way to a phone's drawer); `features/workspace/src/host/group.rs` `Tab` (a tab sliding along its bar, `apps/lab/media/tests/tab_slide.rs`) |
| `overflow: hidden` clips what is drawn, not what is hit: a child scrolled out of its box still takes presses where it would be (over a neighbour). The text of a `position: relative; overflow: hidden` span too: in the fork an overflow box is a stacking context, so its overflowing text is tried before a static neighbour drawn over it (a tab's name took the press meant for its × button). | Draw only what is in the window, and put a neighbour that must win above with `position: relative; z-index: 1` (Blitz tries positive z-indexed children first). | `features/shell/src/shell/bar.rs` `in_view`; `features/workspace/src/host/group.rs` `Tab` (the close button) |
| A scroller keeps its offset when what it holds shrinks (a search narrowing a list): it shows the empty space below the rows. And `onresize` never fires (dioxus-native leaves `convert_resize_data` unimplemented). | Give the scroller a key that changes with what it holds (a `for` over one key makes a new node, at its top); measure a box with `onmounted` + `get_client_rect`, polled with `peek`. | `features/extensions/src/page.rs` `ManagerPage`, `ManagerView` |
| (Fixed in the fork since `fts-measure-2026-11`, carried onto Blitz `main` in `fts-upstream-2026-11`; upstream still has it.) An absolutely placed element inserted into a flex box beside a pane (a 1 px helper, a grip along the pane's foot), or a layer over the screen while a pane mounts under it, made the next layout collapse the pane: a note wrapped a word per line, a centred empty state drew nothing, and it stayed so. The cause: a measure of a text's height at another width (a flex parent sizing a child) broke its lines at that width and left them so, and Taffy then served the text's full layout from its cache without laying the lines out again. A measure now puts the lines back. | Nothing: insert and remove such elements freely. On an older Blitz, keep them mounted and change only their style. | `features/workspace/src/host/overlay.rs` `PhoneBody` (the switcher's grip); `apps/lab/media/tests/reflow.rs` checks it in the films |
| An element removed and another made in the next frame can be drawn with the first's running CSS animation: a pane made as `MotionFrames`' div (an opacity animation, 0–0.01) went showed blank or faint. | Do not mount and unmount an animated element; turn its `animation` on and off. | `features/shell/src/motion.rs` `MotionFrames` |
| `transform: scale(…)` draws a subtree smaller and hit-tests it at that size, at any device scale (a scale has no unit, unlike a `translate`): a pane's own view previewed in a card. | Lay the subtree out at its real size, `transform-origin: 0 0`, and put a box over it (`z-index: 1`) to take the presses. | `features/workspace/src/host/switcher.rs` `CardView` |
| Vertical text (`writing-mode`) is not supported. (Blitz `main` has it behind dioxus-native's `writing-mode` feature, which architect does not turn on.) | An absolutely placed `white-space: nowrap` span with `transform-origin: 0 0; transform: rotate(90deg)`, its `left` half a line further on (the turned line lies left of its origin; not a `translateY`, which is in device pixels); 2D rotations work. | `features/workspace/src/host/group.rs` `Stacked` (the stacked tabs' spines) |

## Fonts

| What happens | Write instead | Where architect does |
|---|---|---|
| A document made with `DioxusDocument::new` and a default `DocumentConfig` has no HTML parser: `dangerous_inner_html` makes no nodes, silently (an editor's painted caret, a widget's markup, an icon's SVG string vanish; the element itself is there, empty). `dioxus_native::launch` sets one; a launcher of its own does not. | Make window documents with `architect_workspace::native::document_config(net, fonts)` (Blitz's `HtmlProvider` in it), as `native::launch` does for the main window and pop-outs. | `features/workspace/src/native.rs` `document_config`; `features/workspace/tests/inner_html.rs` |
| An `@font-face` whose `src` is a `data:` URI is never fetched (an `http:` one is), with or without `format()`, at any size, and with no error. The text falls back to the next family. | Register the font file in the document's `FontContext` before launch: `architect_workspace::native::launch_with_fonts(app, window, fonts)` for a desk app (every window, pop-outs too), `dioxus_native::Config::with_font_ctx` otherwise. Name the family as the file does (Inter: `Inter`). | `features/workspace/src/native.rs` `launch_with_fonts`; `apps/lab/ui/src/lib.rs` `FONTS`, `font_context` |

To check a face really loaded, put a different generic last in the stack for one run (`Inter, serif`): serif text means the face did not resolve.

## Keys and focus

| What happens | Write instead | Where architect does |
|---|---|---|
| Keys reach the DOM only through what has the focus: with nothing focused (after a click on empty space), a page's `onkeydown` hears nothing. Tab moves focus only on from an element that already has it, and a focus change does not reliably reach `onfocus`. | Hear the window's keys: mount `architect_commands::WindowKeys` once, and give chords to commands (⌘ or ⌃). Keep a list's selection in a signal and move it with commands (⌃N, ⌃P), not with focus. | `features/commands/src/native.rs` `WindowKeys`; `apps/lab/ui/src/lib.rs` `Lab::step` |
| Plain keys (↑, ↓, Return) for a list have the same problem, and `WindowKeys` leaves unchorded keys to the page on purpose (the palette's field, typing). | Hear them from the window in a component of the page's own (`dioxus_native::use_window_event`), only while the list is on show and nothing is over it (the palette, settings). | `apps/lab/ui/src/lib.rs` `ListKeys` |
| A click's default action clears the focus when nothing focussable is under it (`blitz-dom`'s `handle_click`), as a press does on `pointerdown`: a button that is not focussable (`tabindex: "-1"`) takes the focus off the field being typed in, and a phone's keyboard goes down. | To keep the field's focus, `prevent_default` both the `pointerdown` and the `click` (and `mousedown`, for a browser). Leave them alone where letting the field go is the point (the hide-keyboard button). | `features/shell/src/shell/bar.rs` `KeyboardBar`, `EditButton` |
| An `autofocus` field is focused before it is a field Blitz types in, so it never enables the window's IME: on iOS the keyboard stays down, though the document reports the text focus. | Raise the keyboard when the text focus arrives (`architect_shell::set_typing` on iOS makes winit's view the first responder). | `features/shell/src/shell/keyboard/ios.rs` `raise_keyboard` |
| A field given a value from code (`value: "{query}"` set before it is focused) keeps its caret at the start, and nothing moves it: what the person types goes in front of the text. | Do not pre-type a field. Narrow what it lists instead and say so in its prompt. | `features/commands/src/ui.rs` `Commands::show_group` |

## The window (macOS)

| What happens | Write instead | Where architect does |
|---|---|---|
| With a transparent, full-size title bar, AppKit moves the window for a drag that starts anywhere in its title bar, over the page or not, and swallows the release: a tab dragged in the top row moved the window. | Make the window not movable by AppKit (`setMovable(false)`) and move it from the row's empty space yourself (`drag_window`, which is `performWindowDrag`). | `features/shell/src/shell/mac.rs` `fit_traffic_lights`; `features/shell/src/shell/mod.rs` `DragStrip` |
| `window.fullscreen()` still reports full screen during the resize that leaves it, so a title row read while drawing stays without its traffic lights. | Follow full screen through the window's events (`use_window_event`), into a signal. | `apps/lab/ui/src/screen.rs` `use_window_chrome` |
| AppKit lays its title bar out again on every resize and on entering or leaving full screen. | Place the traffic lights again after each, and leave AppKit's own bar alone in full screen. | `features/shell/src/shell/mac.rs` `fit_traffic_lights` |
| A Dock badge set while the app is an accessory, or in the same turn it becomes a regular app again, is not drawn on the tile the Dock makes, and setting the same label again draws nothing. | Put the badge back a run-loop turn later, clearing it first (`performSelector:withObject:afterDelay:`). | `features/presence/src/mac.rs` `restore_tile` |
| winit registers no `NSApplicationDelegate`, so reopen on a Dock click, the Dock menu and URLs to open are the app's to hear. | Set one delegate for the app (`architect-presence` does; a second one replaces it). | `features/presence/src/mac.rs` `Delegate` |

## Accessibility

AccessKit is compiled in: `dioxus-native`'s default features (which `architect-commands` takes) include `accessibility`, and so do `blitz-shell`'s. The adapter starts when an assistive tool first asks, and Blitz then builds a tree of every DOM node (`blitz-dom`'s `build_accessibility_node`). Since Blitz `main` (the fork's `fts-upstream-2026-11`) a node is named from `aria-label` (and `aria-labelledby`), its role is read from the `role` attribute before the element's own, and `aria-expanded`, `aria-selected`, `aria-disabled`, `aria-level` and `aria-description` reach the tree. What it still lacks:

| What Blitz's tree lacks | So | What the fix in Blitz is |
|---|---|---|
| Bounds: no node gets `set_bounds` from its layout. | Nothing can be found by position, and macOS drops frameless nodes. | Set each node's bounds from its final layout box. |
| Names from text: text becomes a `TextRun` child that labels its parent, and the macOS adapter hides text runs. | A button named only by its text is unnamed; one with an `aria-label` is named. | Name a node from its text when it has no `aria-label`; give text the `Label` role. |
| Actions: `ActionRequested` is a `TODO` in `blitz-shell`'s `application.rs`. | VoiceOver's press does nothing, and a background click from a testing tool lands on the window, not the page. | Turn a `Click` action into a click on the node. |

None of this can be fixed from architect: the tree is built from the DOM inside Blitz. Architect writes `aria-label` on its icon-only controls (the shell's, the lab's row circles), and those are now named. The rest belongs in the FastTrackStudios Blitz fork (`blitz-dom`'s `accessibility.rs`, `blitz-shell`'s `application.rs`).

## Testing and review tools

- Because the accessibility tree has no bounds or actions (above), a GUI-testing tool's background click, which goes through the accessibility tree, lands on the window instead of the page. With cua-driver, click and drag with `delivery_mode: "foreground"` (real HID events), at pixel positions read from a screenshot. Its first read of the window can list no page nodes at all: that read is what starts the adapter.
- A window capture of a full-screen Blitz window can show a blank band along the top and right that is not on the screen. Check with a capture of the whole display.
- A synthetic click (cua-driver's, at a pixel) on a row of an open `NSMenu` (the Dock's menu, a status item's) only highlights it; press Return to choose it.
- The impeccable detector (`impeccable detect`) reads only web files (html, jsx, tsx, vue, svelte, astro, css, scss, js, ts). It skips `.rs` silently and exits 0, which looks like a clean scan. To get any deterministic signal from `rsx!`, extract the `style:` literals into an HTML file and scan that, then discount the contrast findings the flattening causes (the parents' colours are lost).
