-- Heavy work handed to its worker: the action waits, the views do not.
local n = (args and args.n) or 200000
local w = worker.spawn("primes", { n = n })
local found = w:join()
host.notify(tostring(found) .. " primes below " .. tostring(n), "success", "Timers sampler")
return found
