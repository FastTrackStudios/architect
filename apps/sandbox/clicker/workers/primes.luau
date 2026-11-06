-- The clicker's heavy job, a worker: counts the primes below `args.n` the
-- slow way, in a Luau state of its own on the host's pool, off every
-- view's thread. It shares nothing with the window that spawned it: what
-- it knows comes in `args`, what it finds goes back as progress and its
-- result. Cancelled at its next step when the window goes.
local n = args.n
local found = 0
local every = math.max(1, n // 200)
for i = 2, n do
  local prime = true
  local d = 2
  while d * d <= i do
    if i % d == 0 then prime = false break end
    d += 1
  end
  if prime then found += 1 end
  if i % every == 0 then worker.progress(i, n, found .. ' primes so far') end
end
return found
