-- A two-column table at the cursor, on lines of its own; its first
-- cell's text selected.
local ed = host.editor()
local at = ed.cursor
local before = ""
if at > 0 and string.sub(ed.text, at, at) ~= "\n" then
  before = "\n"
end
local tbl = before .. "| A | B |\n|---|---|\n|   |   |\n"
local first = at + #before + 2
host.edit_editor({
  changes = { { from = at, to = at, insert = tbl } },
  selection = { { anchor = first, head = first + 1 } },
  name = "Insert a table",
  base = ed.version,
})
return true
