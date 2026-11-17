---
"architect-app" = "minor"
"architect-command-palette" = "minor"
"architect-commands" = "minor"
---

Improved ⌘K and ⌘O: the row under the pointer is the picked one, a row picked with the arrow keys is kept in sight (architect_commands::kit::Reveal, and CommandRow's onhover), ⌘K opens on the commands run last under "Recent" (kept across launches in the app's palette-recents.json) then "All commands", ⌘O lists each kind under its heading until something is typed, and a two-line ⌘O row no longer runs into the next.
