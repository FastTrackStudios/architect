-- Fails part-way: it completes the first open task, then asks for a task
-- that does not exist. The error names line 9, and the first change is
-- taken back with the rest of the step: the board is as it was.
local open = state.query("TASKS_LIST", { open = true })
actions.run("TASKS_COMPLETE", { id = open[1].id })
print("completed", open[1].title)

-- A mistyped id: TASKS_COMPLETE refuses it, and the script stops here.
actions.run("TASKS_COMPLETE", { id = "t99" })
print("never printed")
