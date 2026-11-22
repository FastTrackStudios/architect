-- Its cron hook's action (weekdays at 09:00).
local said = "Good morning from the Timers sampler"
host.notify(said, "info", "Timers sampler")
return said
