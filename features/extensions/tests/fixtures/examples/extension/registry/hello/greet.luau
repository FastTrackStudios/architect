-- EXAMPLE_HELLO_GREET: `args` is what the caller passed (⌘K's form, a
-- model's JSON, another script's table); what it returns is the action's
-- result.
local name = if args and args.name then args.name else "you"
return "Hello, " .. name .. "!"
