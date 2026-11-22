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

-- A flat table as JSON (VAULT_CREATE takes the frontmatter as JSON):
-- strings, numbers, booleans and lists of them.
local function json(v)
  local t = type(v)
  if t == "string" then
    return '"' .. string.gsub(v, '[%c"\\]', function(c)
      if c == '"' then return '\\"' end
      if c == "\\" then return "\\\\" end
      if c == "\n" then return "\\n" end
      return string.format("\\u%04x", string.byte(c))
    end) .. '"'
  elseif t == "number" or t == "boolean" then
    return tostring(v)
  elseif t == "table" then
    if #v > 0 then
      local parts = {}
      for _, x in v do
        table.insert(parts, json(x))
      end
      return "[" .. table.concat(parts, ",") .. "]"
    end
    local keys = {}
    for k in v do
      table.insert(keys, k)
    end
    table.sort(keys)
    local parts = {}
    for _, k in keys do
      table.insert(parts, json(tostring(k)) .. ":" .. json(v[k]))
    end
    return "{" .. table.concat(parts, ",") .. "}"
  end
  return "null"
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

-- A note from a template: its fields asked for, {{date}} today.
local a = args or {}
local template = a.template
if template == nil then
  local list = templates("Templates")
  if #list == 0 then
    host.notify("No templates in Templates/", "warning", "Templater-lite")
    return nil
  end
  local choices = {}
  for _, t in list do
    table.insert(choices, { id = t.path, label = t.title })
  end
  template = host.prompt({ kind = "pick_one", title = "Which template?", choices = choices })
  if template == nil then
    return nil
  end
end
local read = host.run("VAULT_READ", { path = template })
local values = { date = a.date or os.date("%Y-%m-%d") }
for k, v in a.fields or {} do
  values[k] = v
end
if a.title ~= nil then
  values.title = a.title
end
if values.title == nil then
  values.title = host.prompt({ kind = "text", title = "Title?", default = "Untitled" })
  if values.title == nil then
    return nil
  end
end
local text = read.body or ""
for _, v in read.properties or {} do
  if type(v) == "string" then
    text = v .. "\n" .. text
  end
end
for _, name in fields(text) do
  if values[name] == nil then
    values[name] = host.prompt({ kind = "text", title = name .. "?" }) or ""
  end
end
local properties = {}
for k, v in read.properties or {} do
  properties[k] = if type(v) == "string" then fill(v, values) else v
end
-- A title is a file name: no folders, no `..`, nothing a file system
-- refuses.
local name = string.gsub(tostring(values.title), '[/\\:%*%?"<>|%c]', "-")
name = string.gsub(name, "%.%.+", ".")
name = string.match(name, "^[%s%.]*(.-)[%s%.]*$")
if name == "" then
  name = "Untitled"
end
local path = (a.folder or "Inbox") .. "/" .. name .. ".md"
host.run("VAULT_CREATE", {
  path = path,
  body = fill(read.body or "", values),
  properties = json(properties),
})
host.notify("Made " .. path, "success", "Templater-lite")
return path
