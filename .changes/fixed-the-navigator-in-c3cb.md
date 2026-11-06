---
"architect-navigator" = "patch"
---

Fixed the navigator in one pane at a time taking presses meant for what is beside it (the shell's ribbon): the pane slid out of view is hidden, not only clipped, as Blitz's hit test does not clip to overflow.
