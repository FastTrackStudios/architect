local paths = {}
for _, doc in host.list_documents() do
  table.insert(paths, doc.path)
end
return { documents = paths, heard = host.data("heard") or {} }
