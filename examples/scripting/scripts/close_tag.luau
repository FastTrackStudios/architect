-- Completes every open task with a tag (`args.tag`; "home" when it is not
-- given) as one undo step: one Undo brings them all back.
local tag = if args and args.tag then args.tag else "home"

local closed = 0
for _, task in state.query("TASKS_LIST", { tag = tag }) do
  if task.done then
    print("already done:", task.title)
    continue
  end
  actions.run("TASKS_COMPLETE", { id = task.id })
  print("done:", task.title)
  closed += 1
end
return closed
