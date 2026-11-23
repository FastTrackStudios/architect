-- Focus mode as it was kept: the tick and the status item follow it.
local on = host.data("focus") == true
ext.state({ kind = "checked", key = "SHOWCASE_TOOLBAR_FOCUS", text = if on then "on" else "off" })
ext.state({ key = "focus", text = if on then "Focus on" else "Focus off" })
return { on = on }
