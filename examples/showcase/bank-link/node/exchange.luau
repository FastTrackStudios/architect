-- Runs on the org's node (`runs = "node"`), never on a device.
--
-- Trades a public token, which the app got from the bank's link flow, for
-- an access token. The client id and secret are named, not known: the
-- node's host puts them into the headers on a request to one of their
-- hosts (`{{secret:…}}`), and this script cannot read them.
local token = args.public_token
if type(token) ~= "string" or not string.match(token, "^[%w%-_]+$") then
	error("public_token: letters, digits, - and _ only")
end
-- The bank's API. A real extension names its provider here; the showcase
-- calls a local mock. Whatever is given, the host only reaches
-- capabilities.hosts, and sends the secrets only to theirs.
local api = args.api or "http://127.0.0.1:7471"

local r = host.fetch(api .. "/item/public_token/exchange", {
	method = "POST",
	headers = {
		["Content-Type"] = "application/json",
		["PLAID-CLIENT-ID"] = "{{secret:client_id}}",
		["PLAID-SECRET"] = "{{secret:client_secret}}",
	},
	body = '{"public_token":"' .. token .. '"}',
})
if r.status ~= 200 then
	error("the bank refused the exchange (" .. tostring(r.status) .. ")")
end
local item = string.match(r.body, '"item_id"%s*:%s*"([^"]+)"')
if not item then
	error("the bank's answer has no item_id")
end
-- The access token is the bank's key to the account: it stays on the
-- node. Keeping it there for later calls is node-kept data, a follow-up
-- of #583; this example answers only what the app may know.
return { linked = true, item_id = item }
