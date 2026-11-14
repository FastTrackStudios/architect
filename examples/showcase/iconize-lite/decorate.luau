-- Icons and colours on the capability folders (the navigator's metadata,
-- as Iconize keeps it), a badge on the index note.
local folders = {
  { folder = "R Runtimes", icon = "cpu", colour = "#38bdf8" },
  { folder = "L Install and lifecycle", icon = "package", colour = "#a78bfa" },
  { folder = "C Contributions", icon = "puzzle", colour = "#f59e0b" },
  { folder = "H Host services", icon = "server", colour = "#22c55e" },
  { folder = "U Drawing and input", icon = "pen-tool", colour = "#f472b6" },
  { folder = "W The web app", icon = "globe", colour = "#ef4444" },
}
local clear = args and args.clear
-- The navigator names a folder by its vault's mount: `@me/` the person's
-- own vault, `@<org>/` an organization's.
local root = (args and args.root) or "@me"
local done = {}
for _, f in folders do
  local folder = root .. "/" .. f.folder
  host.run("NAVIGATOR_ICON", { folder = folder, icon = if clear then "" else f.icon })
  host.run("NAVIGATOR_COLOUR", { target = "folder:" .. folder, colour = if clear then "" else f.colour })
  table.insert(done, f.folder)
end
if clear then
  host.decorate({ kind = "file", id = "Showcase.md" }, nil)
else
  host.decorate({ kind = "file", id = "Showcase.md" }, { icon = "star", badge = "51", color = "primary" })
  host.notify("Decorated " .. #done .. " folders", "success", "Iconize-lite")
end
return done
