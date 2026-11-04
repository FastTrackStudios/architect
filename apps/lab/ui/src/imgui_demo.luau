-- The Embedded views' Luau ImGui pane: a script of its own, its one
-- window drawn in the pane, in the app's theme. The button counts its presses; the slider and
-- the checkbox set the bar under them.
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('imgui-demo')
local presses, fill, bar = 0, 40, true

local function loop()
  -- No close button: the pane is the window's.
  local visible = ImGui.Begin(ctx, 'Window')
  if visible then
    ImGui.Text(ctx, 'Drawn by a Luau script, with ImGui.')
    -- Its colours are the app's theme: the style follows it, and the
    -- script reads it too.
    ImGui.TextColored(ctx, theme.color('brand'), 'Theme: ' .. theme.id .. ' (' .. theme.scheme .. ')')
    ImGui.Separator(ctx)
    if ImGui.Button(ctx, 'Press me', 120, 28) then presses = presses + 1 end
    ImGui.SameLine(ctx)
    ImGui.Text(ctx, presses == 1 and '1 press' or presses .. ' presses')
    local _
    _, fill = ImGui.SliderInt(ctx, 'Fill', fill, 0, 100, '%d%%')
    _, bar = ImGui.Checkbox(ctx, 'Show the bar', bar)
    if bar then
      ImGui.ProgressBar(ctx, fill / 100, nil, nil, fill .. '%')
    end
    ImGui.End(ctx)
  end
  defer(loop)
end
-- Drawn once as it starts, so the pane has its window before any input
-- (each frame after comes with one).
loop()
