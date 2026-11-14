-- A fence's query, answered by the vault's own actions:
--   base: Bases/Showcase tasks.base   (a base's first view's rows)
--   tag: task                         (the notes with a tag)
--   backlinks: Samples/x.md           (the notes that link to one)
local source = args.source or ""
local kind, arg = string.match(source, "^%s*(%a+)%s*:%s*(.-)%s*$")
if kind == nil then
  error("a query is `base: <path>`, `tag: <tag>` or `backlinks: <path>`")
end
local function name_of(path)
  return string.match(path, "([^/]+)%.%w+$") or path
end
if kind == "base" then
  local out = host.run("BASES_ROWS", { path = arg })
  local rows = {}
  for _, g in (out.rows and out.rows.groups) or {} do
    for _, r in g.rows or {} do
      table.insert(rows, { path = r.path, name = r.name, values = r.values })
    end
  end
  return { kind = kind, columns = (out.rows and out.rows.columns) or {}, rows = rows }
elseif kind == "tag" then
  local rows = {}
  for _, n in host.run("TAGS_NOTES", { tag = arg }) do
    table.insert(rows, { path = n.path, name = name_of(n.path), values = { tags = table.concat(n.tags or {}, ", ") } })
  end
  return { kind = kind, columns = { "tags" }, rows = rows }
elseif kind == "backlinks" then
  local rows = {}
  for _, b in host.run("LINKS_BACK", { path = arg }) do
    local path = if type(b) == "string" then b else (b.path or b.from)
    table.insert(rows, { path = path, name = name_of(path), values = {} })
  end
  return { kind = kind, columns = {}, rows = rows }
end
error("unknown query `" .. kind .. "`: base, tag or backlinks")
