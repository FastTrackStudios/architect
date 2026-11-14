-- One tick of its `every` hook: counted in its data and its status item.
local n = (host.data("ticks") or 0) + 1
host.set_data("ticks", n)
ext.state({ key = "ticks", text = n .. (if n == 1 then " tick" else " ticks") })
return n
