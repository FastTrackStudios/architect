-- A menu at a point (Obsidian's `Menu.showAtPosition`).
local x = (args and args.x) or 24
local y = (args and args.y) or 24
local chosen = host.menu({
  x = x,
  y = y,
  items = {
    { id = "copy", label = "Copy a link" },
    { id = "rename", label = "Rename…" },
    { id = "delete", label = "Delete", disabled = true },
  },
})
if chosen ~= nil then
  host.notify("Chose " .. chosen, "info", "Notify sampler")
end
return chosen
