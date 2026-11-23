-- Focus mode, on or off: the toggle's tick and the status item follow.
local on = host.data("focus") ~= true
host.set_data("focus", on)
ext.state({ kind = "checked", key = "SHOWCASE_TOOLBAR_FOCUS", text = if on then "on" else "off" })
ext.state({ key = "focus", text = if on then "Focus on" else "Focus off" })
return { on = on }
