-- Today, from three extensions, through their services.
local today = os.date("%Y-%m-%d")
local function count(v, key)
  if type(v) ~= "table" then
    return 0
  end
  local list = v[key] or v
  local n = 0
  for _ in list do
    n += 1
  end
  return n
end
local events = host.service("architect.calendar", "list", { from = today, to = today })
local plan = host.service("architect.dayplan", "get", { date = today })
local tasks = host.service("architect.tasks", "list", { view = "today" })
local out = { events = count(events, "events"), blocks = count(plan, "blocks"), tasks = count(tasks, "tasks") }
host.notify(
  string.format("Today: %d events, %d plan blocks, %d tasks", out.events, out.blocks, out.tasks),
  "info",
  "Services sampler"
)
return out
