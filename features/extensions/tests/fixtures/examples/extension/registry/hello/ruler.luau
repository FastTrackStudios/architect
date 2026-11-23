-- The ruler: a tick every few seconds across the timeline, and a tool
-- window to change how many. It runs a frame at a time: `defer(loop)`
-- asks for the next one, and the host runs it when something it draws
-- from changes (the view's size or scroll, the pointer over its window).
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('Hello')

-- A layer over views of kind `example.timeline`, over their items.
local ruler = Canvas.Layer('example.timeline', 'over', 'ruler')

local every = 2 -- seconds between ticks
local labels = true
local greeting = ''

local function draw()
  -- The view's coordinate spaces; nil while no timeline is shown.
  local spaces = Canvas.Spaces(ruler)
  if not spaces then return end
  local first = math.ceil(spaces.time.start / every) * every
  local last = Canvas.XToTime(ruler, spaces.width)
  for t = first, last, every do
    local x = Canvas.TimeToX(ruler, t)
    Canvas.Line(ruler, x, 0, x, spaces.height, 0xF59E0BFF, 1)
    if labels then
      Canvas.Text(ruler, x + 3, 2, t .. ' s', 0xF59E0BFF, 11)
    end
  end
end

local function loop()
  draw()
  -- The tool window: `open` turns false when its close box is pressed.
  local visible, open = ImGui.Begin(ctx, 'Ruler', true)
  if visible then
    local _
    _, every = ImGui.SliderInt(ctx, 'Every', every, 1, 8, '%d s')
    _, labels = ImGui.Checkbox(ctx, 'Labels', labels)
    if ImGui.Button(ctx, 'Greet') then
      -- Its own action, through the Action List like anyone's.
      greeting = actions.run('EXAMPLE_HELLO_GREET', { name = 'ruler' })
    end
    ImGui.Text(ctx, greeting)
    ImGui.End(ctx)
  end
  if open then defer(loop) end
end
defer(loop)
