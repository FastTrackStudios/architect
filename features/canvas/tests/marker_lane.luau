-- A marker lane over the arrangement, and a window to tune it.
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('Markers')
local lane = Canvas.Layer('session.arrange', 'over', 'markers')
local lane_name = if args and args.lane then args.lane else 'Kick'
local height, opacity = 24, 30
local markers = state.query('MARKERS_LIST')

local function draw()
  local spaces = Canvas.Spaces(lane)
  if not spaces then return end
  local top = Canvas.Lane(lane, lane_name) or 0
  local alpha = math.floor(opacity / 100 * 255 + 0.5)
  Canvas.Rect(lane, 0, top, spaces.width, height, 0xFF880000 + alpha)
  for _, m in markers do
    local x = Canvas.TimeToX(lane, m.time)
    Canvas.Line(lane, x, top, x, top + height, 0xFF8800FF, 2)
    Canvas.Text(lane, x + 4, top + 4, m.name, 0xFFFFFFFF, 11)
    local _, clicked = Canvas.Region(lane, 'marker:' .. m.name, x - 4, top, 8 + 6 * #m.name, height)
    if clicked then
      actions.run('TRANSPORT_SEEK', { time = m.time })
    end
  end
end

local function loop()
  draw()
  local visible, open = ImGui.Begin(ctx, 'Marker lane', true)
  if visible then
    local _
    _, height = ImGui.SliderInt(ctx, 'Height', height, 8, 64, '%d pt')
    _, opacity = ImGui.SliderDouble(ctx, 'Opacity', opacity, 0, 100, '%.0f%%')
    ImGui.Text(ctx, #markers .. ' markers')
    ImGui.End(ctx)
  end
  if open then defer(loop) end
end
defer(loop)
