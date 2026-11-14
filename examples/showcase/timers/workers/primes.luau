-- Counts the primes below `args.n` the slow way, in a Luau state of its
-- own off every view's thread, saying how far it got as it goes.
local n = args.n
local found = 0
local every = math.max(1, n // 100)
for i = 2, n - 1 do
  local prime = true
  local d = 2
  while d * d <= i do
    if i % d == 0 then
      prime = false
      break
    end
    d += 1
  end
  if prime then
    found += 1
  end
  if i % every == 0 then
    worker.progress(i, n, found .. " primes so far")
  end
end
return found
