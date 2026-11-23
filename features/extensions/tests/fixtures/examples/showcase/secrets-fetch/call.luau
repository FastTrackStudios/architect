-- A call to its host, the token put in by the host when one is set (the
-- extension writes only its name).
local base = (args and args.base) or "http://127.0.0.1:8765"
local headers = {}
if host.secret("token").set then
  headers.Authorization = "Bearer {{secret:token}}"
end
local r = host.fetch(base .. "/echo", { headers = headers })
host.notify("Echo " .. tostring(r.status) .. ": " .. tostring(r.body), "success", "Secrets and fetch")
return { status = r.status, body = r.body }
