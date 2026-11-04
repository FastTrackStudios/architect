-- A deliberately naughty view: while "Spin" is on, each frame spins for
-- 300 ms on the view's own thread — work that belongs in a worker. The
-- lab keeps its last frame on screen, marked busy, and every other pane
-- answers as ever (EXT_VIEWS shows its slow frames). Untick it to stop.
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('naughty')
local frames = 0
local spinning = true

local function loop()
  if spinning then
    local t = os.clock()
    while os.clock() - t < 0.3 do end
  end
  frames += 1
  local visible = ImGui.Begin(ctx, 'Naughty')
  if visible then
    ImGui.Text(ctx, 'Each frame spins for 300 ms on this view\'s thread.')
    local _, now = ImGui.Checkbox(ctx, 'Spin', spinning)
    spinning = now
    ImGui.TextDisabled(ctx, 'frame ' .. frames)
    ImGui.End(ctx)
  end
  wake_after(spinning and 0 or 250)
  defer(loop)
end
loop()
