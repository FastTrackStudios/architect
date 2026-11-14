-- Write the clipboard, read it back.
local text = (args and args.text) or "Copied by the showcase"
host.clipboard_write(text)
local back = host.clipboard_read()
host.notify("On the clipboard: " .. tostring(back), "success", "Notify sampler")
return back
