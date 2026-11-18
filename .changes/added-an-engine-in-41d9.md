---
"architect-extensions" = "minor"
"architect-scene" = "minor"
---

Added an engine in a process of its own: its values shared in a table both processes map (architect_scene::engine::shared: create, open, open_engine; Linux and macOS), read lock-free at compose, and found through the service the view’s extension [uses] or [requires] (engine::read_from, engine::published); a drag on a pane writes its host. value into that table (EngineTable::write_host), where the engine reads it.
