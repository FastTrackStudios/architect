---
"architect-extensions" = "minor"
---

Added architect_extensions::barrier: runtime views that set data-architect-group and data-architect-epoch are shown together; the host holds a member s frame until every member of its group has drawn that epoch (at most barrier::DEADLINE, 50 ms), so an edit lands in the same frame in an arrangement and its mixer (#891).
