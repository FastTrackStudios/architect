-- What a script can see, changing nothing: the actions it may run, what
-- one of them takes, and the board, read through a read-only action.
local tasks = actions.list("TASKS_")
for _, action in tasks do
  print(action.id, action.effect)
end

-- nil when the caller may not run it (a viewer's script does not see it).
local add = actions.describe("TASKS_ADD")
local open = state.query("TASKS_LIST", { open = true })

local by_tag = {}
for _, task in open do
  by_tag[task.tag] = (by_tag[task.tag] or 0) + 1
end

return {
  actions = #tasks,
  add_takes = if add then add.params.required else nil,
  open = #open,
  by_tag = by_tag,
}
