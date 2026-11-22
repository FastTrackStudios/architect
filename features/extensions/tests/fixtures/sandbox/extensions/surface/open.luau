-- Opens the OSC echo, once: its port.
local open = host.data("echo")
if open then
    return open
end
local port = host.open_port("osc", "127.0.0.1:9010", 9011).port
host.set_data("echo", port)
host.log("echo on " .. port .. ": 127.0.0.1:9010, hearing on 9011")
return port
