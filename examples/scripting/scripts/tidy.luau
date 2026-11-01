-- Clears away what is done, and makes sure there is something to do at
-- work. Saved ("Tidy the board"), it is an action of its own: in the list,
-- in ⌘K, bindable to a key, a tool for an agent.
local removed = 0
for _, task in state.query("TASKS_LIST", { open = false }) do
  actions.run("TASKS_REMOVE", { id = task.id })
  removed += 1
end

local added = nil
if #state.query("TASKS_LIST", { tag = "work", open = true }) == 0 then
  added = actions.run("TASKS_ADD", { title = "Plan next week", tag = "work" }).id
end

return { removed = removed, added = added }
