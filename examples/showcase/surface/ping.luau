-- MIDI and OSC by capability: no hardware, no audio. MIDI only to the
-- app's virtual port, which its capability names.
local VIRTUAL = "Lab Surface"
local names = {}
local has_virtual = false
for _, p in host.ports("midi") do
  table.insert(names, p.name)
  if p.name == VIRTUAL then
    has_virtual = true
  end
end
local sent_midi = false
if has_virtual then
  local m = host.open_port("midi", VIRTUAL).port
  host.port_send(m, { type = "cc", channel = 1, number = 7, value = 100 })
  host.close_port(m)
  sent_midi = true
end
-- OSC to itself: it sends to 127.0.0.1:9031 and hears on 9031; its
-- port.message hook says what came back. The port stays open to hear it.
local o = host.data("osc")
if o == nil then
  o = host.open_port("osc", "127.0.0.1:9031", 9031).port
  host.set_data("osc", o)
end
host.port_send(o, { address = "/showcase/ping", args = { 1 } })
host.notify(
  "MIDI: " .. (if sent_midi then "CC 7 sent to " .. VIRTUAL else "no " .. VIRTUAL .. " port") .. "; OSC sent",
  "info",
  "Surface"
)
return { midi = names, sent_midi = sent_midi, sent_osc = true }
