-- A document event: kept, newest last.
local heard = host.data("heard") or {}
local d = args.detail or {}
table.insert(heard, ctx.event .. " " .. (d.path or ((d.from or "?") .. " -> " .. (d.to or "?"))))
host.set_data("heard", heard)
