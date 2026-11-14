-- The system's file picker (desk; a phone answers `unsupported`, #708).
local path = host.pick({
  kind = "open",
  title = "Pick a note",
  filters = { { name = "Notes", extensions = { "md", "txt" } } },
})
if path == nil then
  host.notify("Nothing picked", "info", "Notify sampler")
  return nil
end
host.notify("Picked " .. path, "success", "Notify sampler")
return path
