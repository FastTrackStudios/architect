-- The clicker's heavy window: counts the primes below three million in a
-- worker (workers/primes.luau) and shows its progress here, live. The
-- spinner and the frame count move every frame, job or no job: this
-- window never waits on the work. Drawn by the lab from the clicker's
-- package (the Vault's "Heavy job" pane), in the app's theme.
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('heavy')
local n = 3000000
local job = nil
local turn = 0
local spin = { '|', '/', '-', '\\' }

local function loop()
  turn += 1
  local status = job and job:status() or 'idle'
  local visible = ImGui.Begin(ctx, 'Primes')
  if visible then
    ImGui.Text(ctx, 'The primes below ' .. n .. ', counted in a worker.')
    if status == 'running' then
      if ImGui.Button(ctx, 'Cancel', 140, 28) then job:cancel() end
    elseif ImGui.Button(ctx, 'Find primes', 140, 28) then
      job = worker.spawn('primes', { n = n })
      status = 'running'
    end
    local p = job and job:progress()
    local fraction = (status == 'done' and 1) or (p and p.fraction) or 0
    ImGui.ProgressBar(ctx, fraction, nil, nil, math.floor(fraction * 100) .. '%')
    if status == 'done' then
      ImGui.TextColored(ctx, theme.color('clicker.count'), job:result() .. ' primes')
    elseif p and p.note then
      ImGui.TextDisabled(ctx, p.note)
    end
    ImGui.Separator(ctx)
    ImGui.TextDisabled(ctx, spin[turn % 4 + 1] .. '  frame ' .. turn .. ' · ' .. status)
    ImGui.End(ctx)
  end
  -- The next frame comes by itself: about 30 a second while the job
  -- runs, slower when idle (its news wakes it too).
  wake_after(status == 'running' and 33 or 250)
  defer(loop)
end
loop()
