-- A base's rows counted by a property, in the order each value is first
-- seen.
local out = host.run("BASES_ROWS", { path = args.base })
local order, counts = {}, {}
for _, g in (out.rows and out.rows.groups) or {} do
  for _, r in g.rows or {} do
    local v = r.values and r.values[args.by]
    local label = if v == nil then "(none)" else tostring(v)
    if counts[label] == nil then
      counts[label] = 0
      table.insert(order, label)
    end
    counts[label] += 1
  end
end
local bars = {}
for _, label in order do
  table.insert(bars, { label = label, count = counts[label] })
end
return { by = args.by, bars = bars }
