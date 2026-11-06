-- Shift-Tab in a list item: up to two spaces of its indent taken off.
local ed = host.editor()
if ed.node.kind ~= "list-item" or ed.node.indent == 0 then
  return false
end
local gone = math.min(2, ed.node.indent)
host.edit_editor({
  changes = { { from = ed.node.line_from, to = ed.node.line_from + gone, insert = "" } },
  name = "Outdent list item",
  base = ed.version,
})
return true
