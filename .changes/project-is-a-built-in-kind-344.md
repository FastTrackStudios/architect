---
"vault-live" = "minor"
"vault-proto" = "minor"
---

Changed: projects are architect's (#344). `PropertySchemaRegistry::with_builtins` now has a `project` kind beside `node`, `person` and `daily` (its `state`, `start`, `due`, `up`, `members`, `icon` and `tags`, extending `node`), and `project.md` is the default shelf marker (`shelf_boundary::DEFAULT_SHELF_MARKERS`, `vault_proto::consts::PROJECT_PAGE`): a folder holding one is a root of its own with no `set_shelf_markers` call. An app that wants no nested roots calls `set_shelf_markers(&[])`. Migration: drop an app's own `project` insert and its `set_shelf_markers(&["project.md"])`.
