---
"architect-review" = "minor"
"architect-threads" = "minor"
"architect-distribution" = "minor"
---

Added Review (`architect-review`, extension `review`, service `architect.review`): comments pinned to a time, a range or a part of the frame of a media item's video or recording, per version, kept as Threads threads; the `review` view (stage, transport, timeline with marker rows, comments; J/K/L/M keys) on the playhead; `REVIEW_OPEN`, `REVIEW_ITEM`, `REVIEW_LIST`, `REVIEW_COMMENT`, `REVIEW_ADD_VERSION`, `REVIEW_CARRY`, `REVIEW_PLAY`. Threads (0.1.1) gains an optional `moment` on a thread (`architect_threads::model::Moment`) and `notify: false` on `THREADS_START`/`THREADS_POST`. The Architect distribution links Review in.
