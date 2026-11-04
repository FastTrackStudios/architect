-- The Luau page's worker (luau-primes.luau spawns it): counts the primes below
-- `args.below` in a Luau state of its own, saying how far it is.
local below = args.below or 100000
local found = 0
for n = 2, below do
	local prime = true
	local i = 2
	while i * i <= n do
		if n % i == 0 then
			prime = false
			break
		end
		i += 1
	end
	if prime then
		found += 1
	end
	if n % 20000 == 0 then
		worker.progress(n, below)
		worker.post({ found = found })
	end
end
return { found = found, below = below }
