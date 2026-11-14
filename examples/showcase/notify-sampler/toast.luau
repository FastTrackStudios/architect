-- A toast (Obsidian's `new Notice(…)`).
local text = (args and args.text) or "Hello from the showcase"
local kind = (args and args.kind) or "info"
host.notify(text, kind, "Notify sampler")
return text
