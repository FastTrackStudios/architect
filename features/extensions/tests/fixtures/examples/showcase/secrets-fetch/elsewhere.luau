-- A host it does not name: refused before anything is sent.
local ok, why = pcall(function()
  return host.fetch("https://example.com/")
end)
-- The refusal's own words, without the script's line and traceback.
local said = if ok
  then "reached example.com (it should not have)"
  else (string.match(tostring(why), "(refused[^\n]*)") or tostring(why))
said = string.gsub(said, "%s*stack traceback.*$", "")
host.notify(said, if ok then "error" else "info", "Secrets and fetch")
return said
