-- Today's date at the cursor, one undo step.
local ed = host.editor()
local date = (args and args.date) or os.date("%Y-%m-%d")
host.edit_editor({
  changes = { { from = ed.cursor, to = ed.cursor, insert = date } },
  name = "Insert the date",
  base = ed.version,
})
return date
