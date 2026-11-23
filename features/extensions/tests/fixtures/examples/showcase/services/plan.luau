-- A task through Tasks and an event through Calendar.
local title = (args and args.title) or "Showcase review"
local start = (args and args.start) or (os.date("%Y-%m-%d") .. "T15:00")
local task = host.service("architect.tasks", "create", { title = title })
local event = host.service("architect.calendar", "create", { title = title, start = start })
host.notify("Planned " .. title, "success", "Services sampler")
return { task = task, event = event }
