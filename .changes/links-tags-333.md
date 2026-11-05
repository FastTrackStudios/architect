---
"links" = "minor"
"links-proto" = "minor"
"tag" = "minor"
"tag-proto" = "minor"
"architect-lab-ui" = "none"
---

Links and tags move into architect from Task (#333): typed links and their store (`links-proto`, `links`, `features/links`) and the tag registry (`tag-proto`, `tag`, `features/tags`). Their `actions` features add `LINKS_*` (the wikilink graph, backlinks, unresolved links, orphans) and `TAGS_*` (tags in use with their look, the notes under a tag) over an open vault, and the lab registers them on its sample vault. `tag` no longer turns on `architect/dispatch-tokio`: the app does. Task's `sync_vault_links` example (scripture) and `links-ui` stay in Task.
