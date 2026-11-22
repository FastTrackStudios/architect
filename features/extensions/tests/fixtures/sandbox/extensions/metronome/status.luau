-- Its own settings, the selection and the app's status, through the
-- host service (`host`): each call checked against its capabilities.
local volume = host.setting("fts.metronome.volume")
local sound = host.setting("fts.metronome.sound")
local selection = host.selection()
local status = host.run("APP_STATUS")
local problems = if status and status.problems then #status.problems else 0
host.set_data("last_volume", volume)
host.log("status read: volume " .. tostring(volume) .. ", " .. tostring(problems) .. " problem(s)")
-- The click, and where it was last turned on or off from (`click.luau`).
local click = host.data("click") == true
local last = host.data("last")
return { volume = volume, sound = sound, selection = selection, problems = problems, click = click, last = last }
