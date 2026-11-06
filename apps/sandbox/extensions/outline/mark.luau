-- The note marked up: each TODO marked, a gutter marker on each list
-- item, and a widget with their count at the end of the text. Positions
-- are byte offsets from 0 (Luau's string positions are from 1).
if args and args.clear then
  host.decorate_editor({})
  return 0
end
local ed = host.editor()
local text = ed.text
local decorations = {}
local at = 1
while true do
  local s, e = string.find(text, "TODO", at, true)
  if not s then
    break
  end
  table.insert(decorations, { kind = "mark", from = s - 1, to = e, class = "todo", title = "To do" })
  at = e + 1
end
local items = 0
local line_start = 0
for line in string.gmatch(text .. "\n", "([^\n]*)\n") do
  if string.match(line, "^%s*[-*+] ") or string.match(line, "^%s*%d+[.)] ") then
    items += 1
    table.insert(decorations, { kind = "gutter", at = line_start, text = tostring(items), class = "item" })
  end
  line_start += #line + 1
end
table.insert(decorations, {
  kind = "widget",
  at = #text,
  view = "count",
  props = { items = items },
  block = true,
})
host.decorate_editor(decorations)
return items
