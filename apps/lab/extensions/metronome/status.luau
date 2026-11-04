-- Its own settings, the selection and the app's status, through the
-- host service (`host`): each call checked against its capabilities.
local volume = host.setting("fts.metronome.volume")
local sound = host.setting("fts.metronome.sound")
local selection = host.selection()
local status = host.run("APP_STATUS")
local problems = if status and status.problems then #status.problems else 0
host.set_data("last_volume", volume)
host.log("status read: volume " .. tostring(volume) .. ", " .. tostring(problems) .. " problem(s)")
return { volume = volume, sound = sound, selection = selection, problems = problems }
