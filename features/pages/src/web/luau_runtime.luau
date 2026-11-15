--!strict
-- A Luau extension's globals in a browser (#812): run in its Web Worker
-- (`web/luau.js`) before its script, on luau-web's Luau.
--
-- What each global does is the page's, in Rust: every call is a message
-- the page answers (`web/luau.rs`) with the one implementation a desk's
-- runtime calls too: `host.*` with `architect_extensions::luau_calls` and
-- the extension's `ExtensionApi` (`WebHost::through`), `ext.*` the same
-- module's `ext_call`, `actions.*` and `state.query` with
-- `architect_scripting::bindings`. Values cross as JSON, decoded into
-- Luau tables here (`null` is `nil`, as a desk's `to_lua` has it).
--
-- Called with the worker's functions and the run's start; answers the
-- globals to set, read-only, and `run`, which runs a loaded script and
-- answers what it returned as JSON.

local call: any, say: any, run_worker: any, start_json: string = ...

-- JSON -------------------------------------------------------------------

local encode: (any, number) -> string

local escapes = {
	['"'] = '\\"',
	["\\"] = "\\\\",
	["\b"] = "\\b",
	["\f"] = "\\f",
	["\n"] = "\\n",
	["\r"] = "\\r",
	["\t"] = "\\t",
}

local function encode_string(s: string): string
	return '"' .. string.gsub(s, '[%c"\\]', function(c)
		return escapes[c] or string.format("\\u%04x", string.byte(c))
	end) .. '"'
end

local function encode_number(n: number): string
	if n ~= n or n == math.huge or n == -math.huge then
		return "null"
	end
	if math.floor(n) == n and math.abs(n) < 2 ^ 53 then
		return string.format("%d", n)
	end
	return string.format("%.17g", n)
end

-- A sequence (keys 1..n, none missing) is an array; an empty table `{}`.
local function is_sequence(t: { [any]: any }): boolean
	local n = 0
	for k in t do
		if type(k) ~= "number" or k < 1 or math.floor(k) ~= k then
			return false
		end
		n += 1
	end
	return n > 0 and n == #t
end

function encode(v: any, depth: number): string
	local kind = type(v)
	if v == nil then
		return "null"
	elseif kind == "boolean" then
		return if v then "true" else "false"
	elseif kind == "number" then
		return encode_number(v)
	elseif kind == "string" then
		return encode_string(v)
	elseif kind == "table" then
		if depth > 128 then
			error("nested too deep to be JSON", 0)
		end
		local parts = {}
		if is_sequence(v) then
			for i = 1, #v do
				parts[i] = encode(v[i], depth + 1)
			end
			return "[" .. table.concat(parts, ",") .. "]"
		end
		for k, item in v do
			local key = if type(k) == "string" then k elseif type(k) == "number" then encode_number(k) else nil
			if key == nil then
				error("a " .. type(k) .. " key is not JSON", 0)
			end
			table.insert(parts, encode_string(key) .. ":" .. encode(item, depth + 1))
		end
		return "{" .. table.concat(parts, ",") .. "}"
	end
	error("a " .. kind .. " is not JSON", 0)
end

local decode_at: (string, number) -> (any, number)

local function skip(s: string, i: number): number
	return (string.find(s, "[^ \t\r\n]", i)) or (#s + 1)
end

local function decode_string(s: string, i: number): (string, number)
	-- `i` is past the opening quote.
	local out = {}
	while true do
		local j = string.find(s, '["\\]', i)
		if not j then
			error("unterminated string in JSON", 0)
		end
		table.insert(out, string.sub(s, i, j - 1))
		if string.sub(s, j, j) == '"' then
			return table.concat(out), j + 1
		end
		local e = string.sub(s, j + 1, j + 1)
		if e == "u" then
			local code = tonumber(string.sub(s, j + 2, j + 5), 16) :: number
			i = j + 6
			if code >= 0xD800 and code <= 0xDBFF and string.sub(s, i, i + 1) == "\\u" then
				local low = tonumber(string.sub(s, i + 2, i + 5), 16) :: number
				code = 0x10000 + (code - 0xD800) * 0x400 + (low - 0xDC00)
				i += 6
			end
			table.insert(out, utf8.char(code))
		else
			local map = { b = "\b", f = "\f", n = "\n", r = "\r", t = "\t" }
			table.insert(out, map[e] or e)
			i = j + 2
		end
	end
end

function decode_at(s: string, i: number): (any, number)
	i = skip(s, i)
	local c = string.sub(s, i, i)
	if c == "{" then
		local t = {}
		i = skip(s, i + 1)
		if string.sub(s, i, i) == "}" then
			return t, i + 1
		end
		while true do
			i = skip(s, i)
			local key
			key, i = decode_string(s, i + 1)
			i = skip(s, i) + 1 -- the ':'
			local value
			value, i = decode_at(s, i)
			t[key] = value
			i = skip(s, i)
			local d = string.sub(s, i, i)
			i += 1
			if d == "}" then
				return t, i
			end
		end
	elseif c == "[" then
		local t = {}
		local n = 0
		i = skip(s, i + 1)
		if string.sub(s, i, i) == "]" then
			return t, i + 1
		end
		while true do
			local value
			value, i = decode_at(s, i)
			n += 1
			t[n] = value
			i = skip(s, i)
			local d = string.sub(s, i, i)
			i += 1
			if d == "]" then
				return t, i
			end
		end
	elseif c == '"' then
		return decode_string(s, i + 1)
	elseif string.sub(s, i, i + 3) == "true" then
		return true, i + 4
	elseif string.sub(s, i, i + 4) == "false" then
		return false, i + 5
	elseif string.sub(s, i, i + 3) == "null" then
		return nil, i + 4
	end
	local num = string.match(s, "^-?%d+%.?%d*[eE]?[-+]?%d*", i)
	if not num or num == "" then
		error("not JSON at " .. i, 0)
	end
	return tonumber(num), i + #num
end

local function decode(s: string): any
	return (decode_at(s, 1))
end

local start = decode(start_json)

-- Arguments, in place: `nil`s kept as `null`.
local function encode_args(...): string
	local n = select("#", ...)
	local parts = table.create(n)
	for i = 1, n do
		parts[i] = encode((select(i, ...)), 0)
	end
	return "[" .. table.concat(parts, ",") .. "]"
end

local function line(...): string
	local parts = {}
	for i = 1, select("#", ...) do
		parts[i] = tostring((select(i, ...)))
	end
	return table.concat(parts, "\t")
end

local function print(...)
	say(line(...))
end

local function returned(main)
	local out = main()
	local ok, text = pcall(encode, out, 0)
	if not ok then
		error("what it returned is not JSON: " .. tostring(text), 0)
	end
	return text
end

-- A worker's own globals: `args`, what its owner posted before it ran,
-- and its `worker` (as a desk's `crate::worker`, less serving messages).
-- It runs while its owner waits on the page, so it calls out to nothing:
-- what it posts, its progress and its prints are kept here and answered
-- with its result, as one JSON report.
if start.mode == "worker" then
	local inbox = start.inbox or {}
	local outbox, log = {}, {}
	local progress = nil
	local function keep(...)
		table.insert(log, line(...))
		if #log > 100 then
			table.remove(log, 1)
		end
	end
	local own = {
		id = 1,
		name = start.name,
		post = function(message)
			table.insert(outbox, message)
		end,
		progress = function(done: number, total: number?, note: string?)
			progress = {
				done = done,
				total = total,
				fraction = if total and total > 0 then done / total else nil,
				note = note,
			}
		end,
		cancelled = function()
			return false
		end,
		receive = function(_ms: number?)
			return table.remove(inbox, 1)
		end,
		poll = function()
			return table.remove(inbox, 1)
		end,
		on_message = function()
			error("a worker in a browser does not serve messages yet (#812)", 2)
		end,
	}
	table.freeze(own)
	local args = start.args
	if type(args) == "table" then
		table.freeze(args)
	end
	return {
		args = args,
		worker = own,
		print = keep,
		log = keep,
		__run = function(main)
			local ok, out = pcall(main)
			local report = { messages = outbox, progress = progress, log = log }
			if ok then
				local fine, text = pcall(encode, out, 0)
				if fine then
					report.status = "done"
					report.result = out
				else
					report.status = "failed"
					report.error = "what it returned is not JSON: " .. tostring(text)
				end
			else
				report.status = "failed"
				report.error = tostring(out)
			end
			return encode(report, 0)
		end,
	}
end

-- Calls ------------------------------------------------------------------

-- One message to the page, its answer or its refusal raised at the
-- script's line (as a desk's `raise`).
local function ask(group: string, name: string, args: string): any
	local answer: string = call(group, name, args)
	local text = string.sub(answer, 2)
	if string.sub(answer, 1, 1) == "1" then
		return decode(text)
	end
	error(text, 3)
end

local function group(name: string, names: { string }): { [string]: any }
	local t = {}
	for _, fn in names do
		t[fn] = function(...)
			return ask(name, fn, encode_args(...))
		end
	end
	return t
end

local host = group("host", start.host)
host.id = start.id
-- The raw call: its answer as a table, never raised.
host.call = function(request)
	return ask("host", "call", encode_args(request))
end

local ext = group("ext", start.ext)
ext.id = start.id

local actions = group("actions", { "list", "describe", "enabled" })
actions.enabled = function(id)
	local answer = ask("actions", "enabled", encode_args(id))
	return answer[1], answer[2]
end
actions.run = function(id, args)
	return ask("actions", "run", encode_args(id, args))
end
local state = {
	query = function(id, args)
		return ask("state", "query", encode_args(id, args))
	end,
}

-- Workers: a script's heavy work in a Luau state of its own, in this
-- Web Worker. It runs when its owner first waits on it (`join`,
-- `result`, `status`, …), to its end, with what was posted to it by then.
local workers = {}
local next_worker = 0

local function settle(h)
	if h.state ~= nil then
		return
	end
	local answer = decode(run_worker(h.name, encode(h.args, 0), encode(h.inbox, 0)))
	h.state = answer.status
	h.value = answer.result
	h.error = answer.error
	h.outbox = answer.messages or {}
	h.progress = answer.progress
	h.log = answer.log or {}
end

local Handle = {}
Handle.__index = Handle
function Handle:post(message)
	if self.state ~= nil then
		error("the worker has ended", 2)
	end
	table.insert(self.inbox, message)
end
function Handle:cancel()
	if self.state == nil then
		self.state = "cancelled"
	end
end
function Handle:status()
	settle(self)
	return self.state
end
function Handle:join()
	settle(self)
	if self.state == "failed" then
		error(self.error, 2)
	end
	return self.value
end
function Handle:result()
	settle(self)
	return self.value, self.error
end
function Handle:progress()
	settle(self)
	return self.progress
end
function Handle:messages()
	settle(self)
	local out = self.outbox
	self.outbox = {}
	return out
end
function Handle:next()
	settle(self)
	return table.remove(self.outbox, 1)
end
function Handle:log()
	settle(self)
	return self.log
end
local function live_only()
	error("a view's callbacks (on_message, on_progress, on_done) are a live script's", 2)
end
Handle.on_message, Handle.on_progress, Handle.on_done = live_only, live_only, live_only

local worker = {
	spawn = function(name: string, args)
		if not start.workers[name] then
			error("this extension has no worker " .. tostring(name), 2)
		end
		next_worker += 1
		local h = setmetatable({ id = next_worker, name = name, args = args, inbox = {} }, Handle)
		workers[next_worker] = h
		return h
	end,
}
for _, method in { "post", "cancel", "status", "join", "result", "progress", "messages", "next", "log", "on_message", "on_progress", "on_done" } do
	(worker :: any)[method] = function(h, ...)
		return (Handle :: any)[method](h, ...)
	end
end

local args = start.args
local ctx = start.ctx or {}
local modules = { host = host, ext = ext, ctx = ctx }

local function freeze(t)
	if type(t) == "table" and not table.isfrozen(t) then
		table.freeze(t)
	end
end
for _, t in { host, ext, actions, state, worker } do
	freeze(t)
end
freeze(ctx)
freeze(args)

return {
	host = host,
	ext = ext,
	ctx = ctx,
	args = args,
	actions = actions,
	state = state,
	worker = worker,
	print = print,
	log = print,
	require = function(path: string)
		local found = modules[path]
		if found == nil then
			error(string.format("there is no module %q here", path), 2)
		end
		return found
	end,
	-- Runs the script, answering what it returned as JSON.
	__run = returned,
}
