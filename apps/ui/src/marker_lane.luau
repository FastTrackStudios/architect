-- The lab's marker lane: flags over the arrangement at each marker, a
-- wash across one lane, and a tool window whose sliders tune them.
-- Click a flag to put the playhead there.
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('Markers')
local lane = Canvas.Layer('lab.arrange', 'over', 'markers')
local markers = {
  { name = 'Intro', time = 0 },
  { name = 'Verse', time = 8 },
  { name = 'Chorus', time = 24 },
  { name = 'Bridge', time = 40 },
}
local lanes = { 'Drums', 'Bass', 'Keys', 'Vocals' }
local height, opacity, which, playhead = 28, 35, 4, nil

local function draw()
  local spaces = Canvas.Spaces(lane)
  if not spaces then return end
  local top = Canvas.Lane(lane, lanes[which]) or 0
  local alpha = math.floor(opacity / 100 * 255 + 0.5)
  Canvas.Rect(lane, 0, top, spaces.width, height, 0xF59E0B00 + alpha)
  for _, m in markers do
    local x = Canvas.TimeToX(lane, m.time)
    Canvas.Line(lane, x, 0, x, spaces.height, 0xF59E0B88, 1)
    Canvas.Rect(lane, x, top, 10 + 7 * #m.name, 18, 0xF59E0BFF, true, 3)
    Canvas.Text(lane, x + 5, top + 3, m.name, 0x1C1917FF, 11)
    local hovered, clicked = Canvas.Region(lane, 'marker:' .. m.name, x, top, 10 + 7 * #m.name, 18)
    if hovered then
      Canvas.Rect(lane, x, top, 10 + 7 * #m.name, 18, 0xFFFFFFFF, false, 3)
    end
    if clicked then playhead = m.time end
  end
  if playhead then
    local x = Canvas.TimeToX(lane, playhead)
    Canvas.Line(lane, x, 0, x, spaces.height, 0x2DD4BFFF, 2)
  end
end

local function loop()
  draw()
  ImGui.SetNextWindowPos(ctx, 520, 120, ImGui.Cond_FirstUseEver)
  local visible, open = ImGui.Begin(ctx, 'Marker lane', true, ImGui.WindowFlags_AlwaysAutoResize)
  if visible then
    local _
    _, which = ImGui.SliderInt(ctx, 'Lane', which, 1, #lanes, lanes[which])
    _, height = ImGui.SliderInt(ctx, 'Height', height, 12, 56, '%d pt')
    _, opacity = ImGui.SliderDouble(ctx, 'Opacity', opacity, 0, 100, '%.0f%%')
    ImGui.Separator(ctx)
    ImGui.Text(ctx, #markers .. ' markers')
    if playhead then
      ImGui.SameLine(ctx)
      ImGui.TextDisabled(ctx, 'playhead at ' .. playhead .. ' s')
    end
    ImGui.End(ctx)
  end
  if open then defer(loop) end
end
defer(loop)
