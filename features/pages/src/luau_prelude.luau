-- The Luau page runtime (features/pages/src/luau.rs runs it first): the
-- element builder a `.luau` page draws with, as `div():child(...)`.
-- Its types are architect.d.luau.
local host = __host
local handlers = {}
local root = nil
local dirty = true

local Builder = {}
Builder.__index = Builder

local function token(v)
	return (string.gsub(tostring(v), "%$([%w_][%w_%.%-]*)", function(t)
		return "var(--" .. string.gsub(t, "%.", "-") .. ")"
	end))
end

local function el(tag)
	return setmetatable({ tag = tag, _attrs = {}, _kids = {}, _fns = {} }, Builder)
end

local function text(s)
	return { tag = "#text", text = tostring(s) }
end

function Builder:child(c)
	if type(c) ~= "table" then
		c = text(c)
	end
	table.insert(self._kids, c)
	return self
end

function Builder:children(list)
	for _, c in list do
		self:child(c)
	end
	return self
end

function Builder:key(k)
	self._key = tostring(k)
	return self
end

function Builder:attr(name, value)
	table.insert(self._attrs, { name = name, value = tostring(value) })
	return self
end

function Builder:class(c)
	return self:attr("class", c)
end

function Builder:style(s)
	if type(s) == "table" then
		local parts = {}
		for k, v in s do
			local name = string.gsub(k, "_", "-")
			if type(v) == "number" then
				v = tostring(v) .. "px"
			end
			table.insert(parts, name .. ": " .. token(v))
		end
		table.sort(parts)
		s = table.concat(parts, "; ")
	end
	return self:attr("style", token(s))
end

function Builder:on(event, fn)
	table.insert(self._fns, { event = event, fn = fn })
	return self
end

function Builder:on_click(fn)
	return self:on("click", fn)
end

function Builder:on_input(fn)
	return self:on("input", fn)
end

function Builder:on_key_down(fn)
	return self:on("keydown", fn)
end

-- A builder into the plain tree, its handlers numbered for this render.
local function plain(node)
	if node.tag == "#text" then
		return node
	end
	local on = {}
	for _, h in node._fns do
		table.insert(handlers, h.fn)
		table.insert(on, { event = h.event, handler = #handlers - 1 })
	end
	local kids = {}
	for _, c in node._kids do
		table.insert(kids, plain(c))
	end
	return { tag = node.tag, key = node._key, attrs = node._attrs, on = on, children = kids }
end

local page = {}

function page.render()
	if root == nil then
		error("the page never called render(fn)")
	end
	dirty = false
	handlers = {}
	local made = root()
	if made == nil then
		return {}
	end
	return { plain(made) }
end

function page.event(handler, event)
	local fn = handlers[handler + 1]
	if fn then
		fn(event)
	end
	dirty = true
end

function page.dirty()
	return dirty
end

-- Timers, as a TSX page's setTimeout / setInterval: run by the page's
-- thread (page.tick), each a frame of its own.
local timers = {}
local next_timer = 1

local function add_timer(fn, ms, every)
	local id = next_timer
	next_timer += 1
	local wait = math.max(0, tonumber(ms) or 0)
	timers[id] = { fn = fn, at = host.now() + wait, every = if every then math.max(wait, 1) else 0 }
	return id
end

local function clear_timer(id)
	timers[id] = nil
end

-- Runs the timers that are due; how many ms until the next (-1: none).
function page.tick()
	local now = host.now()
	local due = {}
	for id, t in timers do
		if t.at <= now then
			table.insert(due, id)
		end
	end
	table.sort(due)
	for _, id in due do
		local t = timers[id]
		if t then
			if t.every > 0 then
				t.at = now + t.every
			else
				timers[id] = nil
			end
			dirty = true
			t.fn()
		end
	end
	local soonest = -1
	for _, t in timers do
		if soonest < 0 or t.at < soonest then
			soonest = t.at
		end
	end
	if soonest < 0 then
		return -1
	end
	return math.max(0, soonest - host.now())
end

local api = {
	el = el,
	text = text,
	render = function(fn)
		root = fn
		dirty = true
	end,
	invalidate = function()
		dirty = true
	end,
	set_timeout = function(fn, ms)
		return add_timer(fn, ms, false)
	end,
	set_interval = function(fn, ms)
		return add_timer(fn, ms, true)
	end,
	clear_timeout = clear_timer,
	clear_interval = clear_timer,
}
for _, tag in { "div", "span", "button", "input", "p", "ul", "li", "h1", "h2", "h3", "label", "a", "img", "section" } do
	api[tag] = function()
		return el(tag)
	end
end

return api, page
