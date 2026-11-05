-- What a port it opened heard: answered, /echo<its address>.
return {
    heard = function(event)
        local d = event.detail
        local m = d.message
        if d.kind == "osc" then
            host.port_send(d.port, { address = "/echo" .. m.address, args = m.args })
        end
        ext.state{ key = "heard", text = d.kind .. " " .. (m.address or m.type or "?") }
    end,
}
