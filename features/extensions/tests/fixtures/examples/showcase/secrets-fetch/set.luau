-- The token: asked for once, in the host's own prompt; never readable.
if host.secret("token").set then
  return true
end
return host.ask_secret("token")
