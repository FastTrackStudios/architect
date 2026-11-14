-- {{name}} fields: every one in `text`, once each, in order.
local function fields(text)
  local seen, out = {}, {}
  for name in string.gmatch(text, "{{%s*([%w_]+)%s*}}") do
    if not seen[name] then
      seen[name] = true
      table.insert(out, name)
    end
  end
  return out
end

-- `text` with each {{name}} replaced by `values[name]` (left as it is
-- when there is none).
local function fill(text, values)
  return (string.gsub(text, "{{%s*([%w_]+)%s*}}", function(name)
    local v = values[name]
    if v == nil then
      return nil
    end
    return tostring(v)
  end))
end

-- The templates: each note in `folder`, and the fields it asks for.
local function templates(folder)
  local out = {}
  for _, note in host.run("VAULT_LIST", { folder = folder }) do
    local read = host.run("VAULT_READ", { path = note.path })
    local text = read.body or ""
    for _, v in read.properties or {} do
      if type(v) == "string" then
        text = v .. "\n" .. text
      end
    end
    -- Its file's name: a template's own title is a field.
    local name = string.match(note.path, "([^/]+)%.md$") or note.path
    table.insert(out, { path = note.path, title = name, fields = fields(text) })
  end
  return out
end

return templates((args and args.folder) or "Templates")
