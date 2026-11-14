-- A capture made into a capability note in Inbox/.
local path = "Inbox/" .. args.title .. ".md"
host.run("VAULT_CREATE", {
  path = path,
  body = "# " .. args.title .. "\n\n" .. (args.body or "") .. "\n\n## Try\n\n1. \n\n## You should see\n\n",
  properties = '{"tags":["showcase"]}',
})
return { path = path }
