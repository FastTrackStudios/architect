-- A count-in: `bars` bars of four, the downbeat of each accented.
local bars = if args and args.bars then args.bars else 1
local clicks = {}
for _ = 1, bars do
  for beat = 1, 4 do
    table.insert(clicks, if beat == 1 then "accent" else "click")
  end
end
print("counting in", bars, "bar(s)")
return clicks
