---
"architect-cli" = "minor"
"architect-sites" = "minor"
---

Added scoped Sites: `Options::scope` (`Scope`: a folder rooted at its own note, a tag, a note list, or a base's rows); a link to a note outside it is plain text, an embed of one is left out, and no backlink, base row, search entry or attachment reaches it (under a folder scope, only the folder's attachments). `architect site build --folder/--tag/--note/--base-scope` exports one.
