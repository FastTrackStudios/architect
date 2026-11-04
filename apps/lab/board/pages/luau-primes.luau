-- The Board's primes, as a Luau page (docs/content/architecture/pages.md):
-- a worker (count.luau, a file of this folder) counts while the page keeps
-- answering; a timer ticks so you can see it does. Its types are
-- <app dir>/architect.d.luau.
local state = "Not counted yet."
local done, total = 0, 0
local job = nil
local ticks = 0

set_interval(function()
	ticks += 1
end, 500)

local function count()
	state = "Counting in a worker…"
	done, total = 0, 0
	job = worker.spawn("count.luau", { below = 400000 })
	job:on_progress(function(p)
		done, total = p.done, p.total or 0
	end)
	job:on_message(function(m)
		state = "Found " .. m.found .. " so far…"
	end)
	job:on_done(function(result, err)
		job = nil
		if err then
			state = "Stopped: " .. err
		elseif result then
			state = result.found .. " primes below " .. result.below
		end
	end)
end

render(function()
	local pct = if total > 0 then math.floor(done / total * 100) else 0
	return div()
		:style({ padding = 16, display = "flex", flex_direction = "column", gap = 10 })
		:child(h2():style({ margin = 0 }):child("Primes, as a Luau page"))
		:child(
			p():style({ margin = 0, color = "$muted-foreground", font_size = 12 })
				:child("pages/luau-primes.luau: a worker counts while this page keeps answering. Ticks: " .. ticks)
		)
		:child(
			div()
				:style({ height = 8, border_radius = 4, background = "$muted", overflow = "hidden" })
				:child(div():style({ height = 8, width = pct .. "%", background = "$primary" }))
		)
		:child(span():class("primes-state"):child(state))
		:child(
			button()
				:style({ align_self = "flex-start", padding = "5px 12px", border_radius = 6 })
				:child(if job then "Cancel" else "Count primes")
				:on_click(function()
					if job then
						job:cancel()
					else
						count()
					end
				end)
		)
end)
