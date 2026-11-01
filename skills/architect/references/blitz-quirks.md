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
| In a dock, a flex child's wrapping text measures at its narrowest (min-content), so a centred empty state wraps a word per line. | Lay the block against the pane's own edges: `position: absolute; top: 0; left: 0; right: 0; bottom: 0`, then flex inside it. | `apps/lab/ui/src/vault.rs` `Stand` |
| A space where two spans meet is dropped, even a non-breaking one (`&nbsp;`, `\u{a0}`): "a **b** c" reads "a**b**c". | Trim each run and turn its edge spaces into margin on `display: inline-block` runs, about 0.27em a space. | `features/commands/src/ui.rs` `Lit` |
| The `ch` unit is not supported. | Size in `px` or `em`. | |
| `placeholder` on an input draws nothing. | Put the question in a `label` above the field. | `apps/lab/ui/src/pages.rs` `CapturePage` |
| An element measured in `onmounted` can report a zero rect: it is not laid out yet, and nothing measures it again. | Measure again when the number is needed (a menu opening, a drag starting), and ignore a zero size. | `features/workspace/src/host/mod.rs` `Host::open_menu` |
| Vertical text (`writing-mode`) is not supported. | An absolutely placed `white-space: nowrap` span with `transform-origin: 0 0; transform: rotate(90deg) translateY(-half its line)`; 2D transforms work. | `features/workspace/src/host/group.rs` `Stacked` (the stacked tabs' spines) |

## Fonts

| What happens | Write instead | Where architect does |
|---|---|---|
| An `@font-face` whose `src` is a `data:` URI is never fetched (an `http:` one is), with or without `format()`, at any size, and with no error. The text falls back to the next family. | Register the font file in the document's `FontContext` before launch: `architect_workspace::native::launch_with_fonts(app, window, fonts)` for a desk app (every window, pop-outs too), `dioxus_native::Config::with_font_ctx` otherwise. Name the family as the file does (Inter: `Inter`). | `features/workspace/src/native.rs` `launch_with_fonts`; `apps/lab/ui/src/lib.rs` `FONTS`, `font_context` |

To check a face really loaded, put a different generic last in the stack for one run (`Inter, serif`): serif text means the face did not resolve.

## Keys and focus

| What happens | Write instead | Where architect does |
|---|---|---|
| Keys reach the DOM only through what has the focus: with nothing focused (after a click on empty space), a page's `onkeydown` hears nothing. | Hear the window's keys: mount `architect_commands::WindowKeys` once, and give chords to commands. | `features/commands/src/native.rs` `WindowKeys` |

## The window (macOS)

| What happens | Write instead | Where architect does |
|---|---|---|
| With a transparent, full-size title bar, AppKit moves the window for a drag that starts anywhere in its title bar, over the page or not, and swallows the release: a tab dragged in the top row moved the window. | Make the window not movable by AppKit (`setMovable(false)`) and move it from the row's empty space yourself (`drag_window`, which is `performWindowDrag`). | `features/shell/src/shell/mac.rs` `fit_traffic_lights`; `features/shell/src/shell/mod.rs` `DragStrip` |
| `window.fullscreen()` still reports full screen during the resize that leaves it, so a title row read while drawing stays without its traffic lights. | Follow full screen through the window's events (`use_window_event`), into a signal. | `apps/lab/ui/src/screen.rs` `use_window_chrome` |
| AppKit lays its title bar out again on every resize and on entering or leaving full screen. | Place the traffic lights again after each, and leave AppKit's own bar alone in full screen. | `features/shell/src/shell/mac.rs` `fit_traffic_lights` |

## Testing and review tools

- The page has no accessibility tree (the `dioxus-native` features architect enables leave out `accessibility`). VoiceOver sees an empty window, and a GUI-testing tool's background click, which goes through the accessibility tree, lands on the window instead of the page. With cua-driver, click and drag with `delivery_mode: "foreground"` (real HID events), at pixel positions read from a screenshot.
- A window capture of a full-screen Blitz window can show a blank band along the top and right that is not on the screen. Check with a capture of the whole display.
- The impeccable detector (`impeccable detect`) reads only web files (html, jsx, tsx, vue, svelte, astro, css, scss, js, ts). It skips `.rs` silently and exits 0, which looks like a clean scan. To get any deterministic signal from `rsx!`, extract the `style:` literals into an HTML file and scan that, then discount the contrast findings the flattening causes (the parents' colours are lost).
