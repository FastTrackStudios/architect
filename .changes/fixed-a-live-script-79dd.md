---
"architect-scripting" = "patch"
---

Fixed a live script's start, like each of its frames, being bounded by `Limits::frame` (its time) instead of `Limits::steps`: a view spinning a few hundred ms no longer hits the step limit on fast machines (Apple silicon), so it shows busy as on Linux. `Limits::steps` bounds one-off runs only.
