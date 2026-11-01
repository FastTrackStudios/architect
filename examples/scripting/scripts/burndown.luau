-- A live script (run it with `live`): its first run only hands `tick` to
-- `defer`; each frame after completes one open task, as an undo step of
-- its own, until none is left.
local frame = 0

local function tick()
  frame += 1
  local open = state.query("TASKS_LIST", { open = true })
  if #open == 0 then
    print("frame", frame, "nothing left")
    return
  end
  actions.run("TASKS_COMPLETE", { id = open[1].id })
  print("frame", frame, "done:", open[1].title)
  defer(tick)
end

defer(tick)
return "started"
