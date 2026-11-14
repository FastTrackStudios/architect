-- A text prompt, a pick-one list and a confirm (Templater's
-- tp.system.prompt and suggester, QuickAdd's inputs).
local name = host.prompt({ kind = "text", title = "Your name?", default = "Ada" })
if name == nil then
  return nil
end
local colour = host.prompt({
  kind = "pick_one",
  title = "A colour?",
  choices = {
    { id = "teal", label = "Teal" },
    { id = "amber", label = "Amber" },
    { id = "violet", label = "Violet" },
  },
})
local sure = host.prompt({ kind = "confirm", title = "Show it?" })
if sure then
  host.notify(name .. " likes " .. tostring(colour), "success", "Notify sampler")
end
return { name = name, colour = colour, sure = sure }
