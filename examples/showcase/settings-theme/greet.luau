-- The settings page's values, read as the extension; a count kept in its
-- data (a file of its own: it survives a restart).
local text = host.setting("showcase.dusk.greeting") or "Hello"
if host.setting("showcase.dusk.loud") then
  text = string.upper(text)
end
local times = (host.data("times") or 0) + 1
host.set_data("times", times)
host.notify(text, "info", "Settings and theme")
return { text = text, times = times }
