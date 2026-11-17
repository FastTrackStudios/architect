---
"architect-workspace" = "minor"
---

Cursors for a view that draws its own: CURSORS_SELECT publishes this person's selection in a space, CURSORS_ROSTER tells each person's selections, hooks hear changed:cursors when the roster moves, and CURSORS_SPACE_SET custom: true leaves the pointer and the drawing to the view (ViewSpace::Custom). PersonHere gains selections. A throttled state change (what a person looks at, a selection) is now flushed on the room's tick.
