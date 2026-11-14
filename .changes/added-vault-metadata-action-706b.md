---
"vault-obsidian" = "minor"
"vault" = "minor"
"architect-extensions" = "minor"
"architect-notify" = "none"
"architect-threads" = "none"
"architect-calendar" = "none"
"architect-dayplan" = "none"
"architect-projects" = "none"
"architect-tasks" = "none"
"architect-time-tracker" = "none"
---

Added `VAULT_METADATA {path} | {folder, limit?, cursor?}`, a read-only action answering a note's parsed metadata by line (headings, list items with their nesting and section, tasks as indices, `^block` ids, inline `key:: value` fields), or a page of a folder's notes, cached by modified time; and `vault_obsidian::metadata`, its parser: what Dataview- and Tasks-style extensions read (#710 H3).

Changed: an extension runs a `VAULT_*` action only with `reads = ["documents"]` (read-only ones) or `writes = ["documents"]` (the rest), past its `actions` grant (`capabilities::VAULT_ACTIONS`). Migration: an extension that runs `VAULT_*` adds those to its `[capabilities]`; the first-party ones (Calendar, Day Plan, Projects, Tasks, Time Tracker, Notify, Threads) do.
