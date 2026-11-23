-- ⌘O's rows: capability notes (a file named "<id> <title>.md" under a
-- group folder) whose name has every word of the query.
local words = {}
for w in string.gmatch(string.lower((args and args.query) or ""), "%S+") do
  table.insert(words, w)
end
local items = {}
for _, note in host.run("VAULT_LIST", {}) do
  local name = string.match(note.path, "([^/]+)%.md$") or ""
  local id = string.match(name, "^([RLCHUW]%d+) ")
  if id then
    local hay = string.lower(name)
    local all = true
    for _, w in words do
      if not string.find(hay, w, 1, true) then
        all = false
        break
      end
    end
    if all then
      table.insert(items, { id = note.path, label = name, detail = note.folder, group = "Showcase" })
    end
  end
end
return { items = items }
