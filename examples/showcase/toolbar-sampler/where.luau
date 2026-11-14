-- Where it was run from, and on what (the invocation).
local said = tostring(ctx.source)
if ctx.id ~= nil then
  said = said .. " on " .. tostring(ctx.id)
end
host.notify("Run from " .. said, "info", "Toolbar sampler")
return { source = ctx.source, target = ctx.target, id = ctx.id, path = ctx.path }
