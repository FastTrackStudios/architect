-- Shift-Tab in a table: the previous cell's text selected.
local ed = host.editor()
local node = ed.node
if node.kind ~= "table" or node.cell == nil then
  return false
end
local cells = node.cells
for i, c in cells do
  if c.row == node.cell.row and c.col == node.cell.col then
    local prev = cells[i - 1]
    if prev then
      host.edit_editor({
        selection = { { anchor = prev.from, head = prev.to } },
        name = "Previous cell",
        base = ed.version,
      })
    end
    return true
  end
end
return false
