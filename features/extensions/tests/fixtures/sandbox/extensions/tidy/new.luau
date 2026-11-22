-- A note in the notes space, its heading its title.
local title = args.title
return host.create_document("notes/" .. title .. ".md", "# " .. title .. "\n")
