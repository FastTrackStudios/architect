+++
title = "Canvas overlays and ImGui-shaped UI"
description = "architect-canvas: a view publishes its coordinate spaces, scripts draw layers over it with hit regions, and ReaImGui-shaped tool windows open as workspace panes."
weight = 52
+++

## What it is and when to use it

A view publishes its coordinate spaces: a `Spaces` with a time axis across
and named lanes down (an arrangement, a timeline). A script or extension then
draws `Layer`s over that kind of view in those coordinates, on a `Plane`
(under the view's content, over it, or on top of everything). The pointer
passes through a layer everywhere except the regions it claims, so a marker
flag can be clicked while the rest of the view still works. Scripts also get
immediate-mode windows shaped like ReaImGui (`ImGui.Begin`,
`ImGui.SliderDouble`, …), each opened as a floating workspace pane.

Use it for marker lanes, labels and highlights over a timeline, and small
tool windows from scripts, without the view knowing about them. Design:
[Extensions](@/architecture/extensions.md), sections on immediate-mode UI for
scripts and on overlays; panes are [Workspace](@/architecture/workspace.md).

<!-- media: canvas-marker-lane — the lab's arrangement with the marker lane drawn over it: dragging the Opacity slider in the tool window, then clicking a flag to seek -->
<!-- media: canvas-tool-window-pane — a script's ImGui window as a floating pane, docked into a group, then closed and the script stopping -->

## A minimal example

A script, from `features/canvas/tests/marker_lane.luau` (shortened):

```lua
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('Markers')
local lane = Canvas.Layer('session.arrange', 'over', 'markers')
local opacity = 30

local function loop()
  local spaces = Canvas.Spaces(lane)
  if spaces then
    for _, m in state.query('MARKERS_LIST') do
      local x = Canvas.TimeToX(lane, m.time)
      Canvas.Line(lane, x, 0, x, 24, 0xFF8800FF, 2)
      local _, clicked = Canvas.Region(lane, 'marker:' .. m.name, x - 4, 0, 8, 24)
      if clicked then actions.run('TRANSPORT_SEEK', { time = m.time }) end
    end
  end
  local visible, open = ImGui.Begin(ctx, 'Marker lane', true)
  if visible then
    local _
    _, opacity = ImGui.SliderDouble(ctx, 'Opacity', opacity, 0, 100, '%.0f%%')
    ImGui.End(ctx)
  end
  if open then defer(loop) end
end
defer(loop)
```

The app, `architect-canvas` with `ui` and `luau`, after `apps/lab/ui/src/arrange.rs`
and `apps/lab/ui/src/vault.rs`:

```rust
// The page: the canvases, the tool windows as panes, the script
let _canvases = architect_canvas::ui::use_canvases();
architect_canvas::ui::use_tool_windows(workspace);           // Signal<Workspace>
use_script_canvas(Runtime::new(list), ActionCaller::app(),
    include_str!("marker_lane.luau").to_owned(), Value::Null,
    "Marker lane".to_owned(), style_from_theme(&theme));
// Views: .view(architect_canvas::ui::window_view())

// The drawn-over view publishes its spaces and puts the layers last
canvases.publish(ARRANGE, Some(&Spaces::sized(w, h)
    .with_time(TimeAxis { start, pixels_per_second: zoom })
    .with_lane(Lane::new("drums", "Drums", 0.0, 48.0))));
rsx! { CanvasLayers { view: ARRANGE, plane: Plane::Over } }
```

## Code tour

1. `features/canvas/src/space.rs` `Spaces` (`sized`, `with_time`,
   `with_lane`), `TimeAxis`, `Lane`: a view's coordinates.
2. `features/canvas/src/layer.rs` `Plane`, `Layer` (`hit`), `Overlays`
   (`publish`, `pointer`, `draw`, `region`): the layers of one frame.
3. `features/canvas/src/imgui.rs` `Ui` (`begin`, `slider_double`, `button`,
   `same_line`), `Input`, `WindowFrame`, `Style`: the immediate-mode windows.
4. `features/canvas/src/luau.rs` `ImGuiModule`, `CanvasModule`: the `ImGui`
   and `Canvas` globals, as scripting modules.
5. `features/canvas/src/luau.rs` `Session` (`start`, `frame`, `output`,
   `publish`, `pointer`): one live script with both modules.
6. `features/canvas/src/ui.rs` `use_canvases`, `Canvases`, `use_script_canvas`:
   a session driven from a coroutine; pointer moves coalesce, each press gets
   its own frame.
7. `features/canvas/src/ui.rs` `CanvasLayers`, `window_view`,
   `use_tool_windows`, `style_from_theme`: layers drawn on Blitz, windows as panes.

The view publishes its `Spaces` through `Canvases::publish`, which hands them
to each running session. A pointer event, a resize or a closed pane becomes
a message; the session then runs one frame: it begins the ImGui and overlay
frames, runs the script's deferred functions (which call `Canvas.*` and
`ImGui.*`), and collects the layers and windows they drew. `CanvasLayers`
draws the layers for its view and plane as positioned boxes, and
`use_tool_windows` opens or closes a floating pane per window.

## Tests to read

- `features/canvas/tests/marker_lane.rs` (with `features/canvas/tests/marker_lane.luau`):
  flags sit at each marker's time; regions take the pointer and the rest
  passes through; dragging the slider changes the opacity; scrolling moves the
  flags; clicking a flag runs `TRANSPORT_SEEK`; closing the window's pane
  stops the script; ReaImGui's shapes hold; an extension overlay under a
  read-only capability is refused the seek.

## Status

Done in architect, with an ImGui subset (text, buttons, checkbox, sliders,
progress bar, `SameLine`, ids, next-window size and position); the lab runs
its marker lane. Not yet: drawing through egui or Dear ImGui behind the same
API (layers are boxes, as Blitz has no texture widget); images and fonts;
layers from Dioxus views; the extension host starting `[[overlays]]` itself.

- **Task** and **Studio** do not use it yet; Studio's arrangement publishing
  its spaces is the first adopter planned.
