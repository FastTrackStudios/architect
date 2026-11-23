-- One transaction, one undo step: front matter on top, the trailing
-- space of the text trimmed, a closing line.
local path = args.path
local text = host.read_document(path)
local trimmed = string.gsub(text, "%s+$", "")
local edits = {
  { insert = { at = 0, text = "---\ntidy: true\n---\n" } },
}
local gone = utf8.len(text) - utf8.len(trimmed)
if gone > 0 then
  -- Positions count characters: past the front matter just inserted.
  table.insert(edits, { delete = { at = 19 + utf8.len(trimmed), len = gone } })
end
table.insert(edits, { append = { text = "\n\n— tidied\n" } })
return host.edit_document(path, edits, "Tidy " .. path)
