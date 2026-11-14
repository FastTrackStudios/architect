-- https://showcase.example/C6 opens the note "C6 …".
local id = string.match(args.url or "", "/([A-Z]%d+)$")
if id == nil then
  return nil
end
for _, note in host.run("VAULT_LIST", {}) do
  local name = string.match(note.path, "([^/]+)%.md$") or ""
  if string.sub(name, 1, #id + 1) == id .. " " then
    return host.open(note.path)
  end
end
return nil
