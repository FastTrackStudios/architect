-- The click, on or off: a toggle (`toggle = true`), so it says which with
-- runtime state of kind `checked` keyed by its id; its tick in menus and
-- its toolbar buttons follow. It keeps where it was turned from (a tab's
-- or a file's context menu, a toolbar: its invocation) for the status
-- action to read. Run as the startup hook, it says what it was left at.
local on = host.data("click") == true
if ctx.event ~= "startup" then
  on = not on
  host.set_data("click", on)
  host.set_data("last", {
    source = ctx.source,
    target = ctx.target,
    id = ctx.id,
    pane = ctx.pane,
    view = ctx.view,
    path = ctx.path,
  })
end
ext.state{ kind = "checked", key = "FTS_METRONOME_CLICK", text = if on then "on" else "off" }
return { on = on }
