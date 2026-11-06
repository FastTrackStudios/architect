-- Tab in a table: the next cell's text selected; past the last cell, a
-- new row, its first cell's caret.
local ed = host.editor()
local node = ed.node
if node.kind ~= "table" or node.cell == nil then
  return false
end
local cells = node.cells
for i, c in cells do
  if c.row == node.cell.row and c.col == node.cell.col then
    local nxt = cells[i + 1]
    if nxt then
      host.edit_editor({
        selection = { { anchor = nxt.from, head = nxt.to } },
        name = "Next cell",
        base = ed.version,
      })
    else
      local cols = node.cell.col + 1
      local row = "\n|" .. string.rep("   |", cols)
      host.edit_editor({
        changes = { { from = node.line_to, to = node.line_to, insert = row } },
        selection = { { anchor = node.line_to + 3, head = node.line_to + 3 } },
        name = "Add a row",
        base = ed.version,
      })
    end
    return true
  end
end
return false
