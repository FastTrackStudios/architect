-- MIDI and OSC by capability: no hardware, no audio.
local names = {}
for _, p in host.ports("midi") do
  table.insert(names, p.name)
end
local sent_midi = false
if #names > 0 then
  local m = host.open_port("midi", names[1]).port
  host.port_send(m, { type = "cc", channel = 1, number = 7, value = 100 })
  host.close_port(m)
  sent_midi = true
end
-- OSC to itself: it sends to 127.0.0.1:9031 and hears on 9031.
local o = host.open_port("osc", "127.0.0.1:9031", 9031).port
host.port_send(o, { address = "/showcase/ping", args = { 1 } })
local heard = {}
local started = os.clock()
while #heard == 0 and os.clock() - started < 1 do
  for _, msg in host.port_receive(o) do
    table.insert(heard, msg.address)
  end
end
host.close_port(o)
local said = string.format(
  "MIDI ports: %s; OSC heard: %s",
  if #names > 0 then table.concat(names, ", ") else "none",
  if #heard > 0 then table.concat(heard, ", ") else "nothing"
)
host.notify(said, "info", "Surface")
return { midi = names, sent_midi = sent_midi, heard = heard }
