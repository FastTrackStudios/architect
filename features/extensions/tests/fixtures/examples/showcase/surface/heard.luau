-- What one of its ports heard (its port.message hook): the event's detail
-- is {port, kind, name, message}.
return {
  heard = function(event)
    local d = event.detail or {}
    local m = d.message or {}
    local what = m.address or m.type or "a message"
    host.set_data("heard", what)
    host.notify("OSC heard: " .. tostring(what), "success", "Surface")
  end,
}
