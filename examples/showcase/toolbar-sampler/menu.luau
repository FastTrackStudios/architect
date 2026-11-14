-- A menu at a point (Obsidian's Menu.showAtPosition).
local chosen = host.menu({
  x = (args and args.x) or 24,
  y = (args and args.y) or 24,
  items = {
    { id = "bold", label = "Bold" },
    { id = "italic", label = "Italic" },
    { id = "strike", label = "Strikethrough", disabled = true },
  },
})
if chosen ~= nil then
  host.notify("Chose " .. chosen, "info", "Toolbar sampler")
end
return chosen
