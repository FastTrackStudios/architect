-- Tab in a list item: two spaces before it, one undo step. The caret
-- keeps its place in the text.
local ed = host.editor()
if ed.node.kind ~= "list-item" then
  return false
end
host.edit_editor({
  changes = { { from = ed.node.line_from, to = ed.node.line_from, insert = "  " } },
  name = "Indent list item",
  base = ed.version,
})
return true
